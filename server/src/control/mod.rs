//! Exposes the local control API.
pub mod auth;
mod calls;
mod discovery;
mod harness;
mod models;
mod network;
mod overview;
mod pricing;
mod service;

use axum::{
    body::{to_bytes, Body},
    extract::State,
    http::{header, Request, Response, StatusCode},
    routing::{any, get, post, put},
    Router,
};
use tower_http::services::ServeDir;
use url::Url;

pub use service::{CallDetail, CallSummary, ControlService, ModelConnectivityResult};

pub fn web_router(service: ControlService, assets: impl AsRef<std::path::Path>) -> Router {
    Router::new()
        .nest_service(
            "/__byok-api__",
            ServeDir::new(assets).append_index_html_on_directories(true),
        )
        .merge(api_router(service))
}

pub fn proxy_web_router(service: ControlService, target: Url) -> Router {
    frontend_proxy_router(target).merge(api_router(service))
}

fn frontend_proxy_router(target: Url) -> Router {
    let state = FrontendProxy {
        client: reqwest::Client::new(),
        target: target.as_str().trim_end_matches('/').to_string(),
    };
    Router::new()
        .route("/__byok-api__/", any(proxy_frontend))
        .route("/__byok-api__/{*path}", any(proxy_frontend))
        .with_state(state)
}

#[derive(Clone)]
struct FrontendProxy {
    client: reqwest::Client,
    target: String,
}

async fn proxy_frontend(
    State(proxy): State<FrontendProxy>,
    request: Request<Body>,
) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/__byok-api__/");
    let mut upstream = proxy
        .client
        .request(parts.method, format!("{}{path}", proxy.target));
    for (name, value) in &parts.headers {
        if name != header::HOST && name != header::CONNECTION {
            upstream = upstream.header(name, value);
        }
    }
    let body = match to_bytes(body, 64 * 1024 * 1024).await {
        Ok(body) => body,
        Err(error) => return proxy_error(error),
    };
    let upstream = match upstream.body(body).send().await {
        Ok(response) => response,
        Err(error) => return proxy_error(error),
    };
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = match upstream.bytes().await {
        Ok(body) => body,
        Err(error) => return proxy_error(error),
    };
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    for (name, value) in &headers {
        if name != header::CONNECTION
            && name != header::TRANSFER_ENCODING
            && name != header::CONTENT_LENGTH
        {
            response.headers_mut().insert(name, value.clone());
        }
    }
    response
}

fn proxy_error(error: impl std::fmt::Display) -> Response<Body> {
    tracing::warn!(%error, "frontend development proxy failed");
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(Body::from("frontend development server is unavailable"))
        .expect("static proxy error response")
}

pub fn api_router(service: ControlService) -> Router {
    Router::new()
        .route(
            "/__byok-api__/api/models",
            get(models::list).post(models::create),
        )
        .route("/__byok-api__/api/models/order", put(models::reorder))
        .route(
            "/__byok-api__/api/models/{model_hash}",
            put(models::update).delete(models::remove),
        )
        .route(
            "/__byok-api__/api/models/{model_hash}/test/{test_id}",
            post(models::test).delete(models::cancel),
        )
        .route(
            "/__byok-api__/api/sub2api/connection",
            get(connection).put(save_connection),
        )
        .route("/__byok-api__/api/sub2api/models", get(discovery::list))
        .route("/__byok-api__/api/overview", get(overview::get))
        .route(
            "/__byok-api__/api/pricing",
            get(pricing::get).put(pricing::put),
        )
        .route("/__byok-api__/api/pricing/estimate", get(pricing::estimate))
        .route("/__byok-api__/api/network", get(network::get))
        .route("/__byok-api__/api/network/ports", put(network::ports))
        .route("/__byok-api__/api/network/proxy", put(network::proxy))
        .route("/__byok-api__/api/network/test", post(network::test))
        .route(
            "/__byok-api__/api/harness/cursor/subscription",
            put(harness::subscription),
        )
        .route("/__byok-api__/api/llm-calls", get(calls::list))
        .route(
            "/__byok-api__/api/route-diagnostics",
            get(calls::diagnostics),
        )
        .route("/__byok-api__/api/llm-calls/{call_id}", get(calls::detail))
        .route(
            "/__byok-api__/api/harness/cursor/status",
            get(harness::status),
        )
        .route(
            "/__byok-api__/api/harness/cursor/ca/initialize",
            post(harness::initialize_ca),
        )
        .route(
            "/__byok-api__/api/harness/cursor/ca/consent",
            post(harness::consent),
        )
        .route(
            "/__byok-api__/api/harness/cursor/ca/uninstall",
            post(harness::uninstall_certificate),
        )
        .route(
            "/__byok-api__/api/harness/cursor/enabled",
            put(harness::set_enabled),
        )
        .route(
            "/__byok-api__/api/harness/cursor/recover",
            post(harness::recover),
        )
        .route(
            "/__byok-api__/api/{*path}",
            any(|| async { StatusCode::NOT_FOUND }),
        )
        .with_state(service)
}
async fn connection(
    State(service): State<ControlService>,
) -> crate::Result<axum::Json<crate::store::Sub2ApiConnection>> {
    Ok(axum::Json(service.connection().await?))
}
async fn save_connection(
    State(service): State<ControlService>,
    axum::Json(input): axum::Json<crate::store::Sub2ApiConnectionInput>,
) -> crate::Result<axum::Json<crate::store::Sub2ApiConnection>> {
    Ok(axum::Json(service.save_connection(input).await?))
}
