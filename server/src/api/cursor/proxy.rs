//! Selects local handling or the configured official Cursor upstream.
use std::time::Instant;

use axum::{
    body::{to_bytes, Body, Bytes},
    extract::Extension,
    http::{header, Request, Response},
};

use crate::Result;

const CURSOR_UPSTREAM: &str = "https://api2.cursor.sh";
pub const UPSTREAM_URL_HEADER: &str = "x-server-upstream-url";

#[derive(Clone)]
pub struct CursorProxy {
    clients: crate::network::NetworkClients,
    upstream: String,
}

pub struct BufferedResponse {
    pub status: axum::http::StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Bytes,
}

impl BufferedResponse {
    pub fn into_response(self) -> Response<Body> {
        let body = self.body.clone();
        self.with_body(body)
    }

    pub fn with_body(mut self, body: Bytes) -> Response<Body> {
        self.headers.insert(
            header::CONTENT_LENGTH,
            body.len()
                .to_string()
                .parse()
                .expect("body length is always a valid header value"),
        );
        let mut response = Response::new(Body::from(body));
        *response.status_mut() = self.status;
        *response.headers_mut() = self.headers;
        response
    }
}

impl CursorProxy {
    pub fn cursor(clients: crate::network::NetworkClients) -> Self {
        Self {
            clients,
            upstream: CURSOR_UPSTREAM.into(),
        }
    }

    async fn client(&self) -> Result<reqwest::Client> {
        self.clients.cursor_client().await
    }
}

pub async fn forward(
    Extension(proxy): Extension<CursorProxy>,
    request: Request<Body>,
) -> Result<Response<Body>> {
    forward_request(&proxy, request).await
}

async fn forward_request(proxy: &CursorProxy, request: Request<Body>) -> Result<Response<Body>> {
    let started = Instant::now();
    let (parts, body) = request.into_parts();
    reject_placeholder(&parts.headers)?;
    let diagnostic_path = parts.uri.path().to_owned();
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |value| value.as_str())
        .to_owned();
    let url = upstream_url(&parts.headers, &proxy.upstream, &path)?;

    let mut headers = parts.headers;
    headers.remove(UPSTREAM_URL_HEADER);
    headers.remove(crate::control::auth::TOKEN_HEADER);
    headers.remove(header::HOST);
    remove_hop_by_hop_headers(&mut headers);

    let client = proxy.client().await?;
    let upstream = client
        .request(parts.method.clone(), url)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await;

    let upstream = match upstream {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(
                method = %parts.method,
                path = diagnostic_path,
                elapsed_ms = started.elapsed().as_millis(),
                error_kind = if error.is_timeout() { "timeout" } else { "connection" },
                "Cursor upstream request failed"
            );
            return Err(error.without_url().into());
        }
    };

    let status = upstream.status();
    let mut response_headers = upstream.headers().clone();
    remove_hop_by_hop_headers(&mut response_headers);
    let mut response = Response::new(Body::from_stream(upstream.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = response_headers;

    tracing::info!(
        method = %parts.method,
        path = diagnostic_path,
        %status,
        elapsed_ms = started.elapsed().as_millis(),
        "forwarded Cursor backend request"
    );
    let diagnostic = crate::store::RouteDiagnostic {
        method: parts.method.to_string(),
        path: diagnostic_path,
        route: "cursor_official".into(),
        stage: "official_response_headers".into(),
        http_status: status.as_u16(),
        duration_ms: started.elapsed().as_millis() as i64,
        ..Default::default()
    };
    if proxy
        .clients
        .store()
        .record_route_diagnostic(&diagnostic)
        .await
        .is_err()
    {
        tracing::warn!("could not persist official response metadata");
    }
    Ok(response)
}

pub async fn forward_buffered(
    proxy: &CursorProxy,
    request: Request<Body>,
) -> Result<BufferedResponse> {
    let (parts, body) = request.into_parts();
    reject_placeholder(&parts.headers)?;
    let path = parts
        .uri
        .path_and_query()
        .map_or("/", |value| value.as_str());
    let url = upstream_url(&parts.headers, &proxy.upstream, path)?;
    let mut headers = parts.headers;
    headers.remove(UPSTREAM_URL_HEADER);
    headers.remove(crate::control::auth::TOKEN_HEADER);
    headers.remove(header::HOST);
    remove_hop_by_hop_headers(&mut headers);
    headers.insert(
        "connect-accept-encoding",
        axum::http::HeaderValue::from_static("identity"),
    );
    headers.insert(
        header::ACCEPT_ENCODING,
        axum::http::HeaderValue::from_static("identity"),
    );
    let body = to_bytes(body, usize::MAX)
        .await
        .map_err(|error| crate::Error::Protocol(format!("cannot read request body: {error}")))?;
    let upstream = proxy
        .client()
        .await?
        .request(parts.method, url)
        .headers(headers)
        .body(body)
        .send()
        .await?;
    let status = upstream.status();
    let mut headers = upstream.headers().clone();
    remove_hop_by_hop_headers(&mut headers);
    let body = upstream.bytes().await?;
    Ok(BufferedResponse {
        status,
        headers,
        body,
    })
}

fn reject_placeholder(headers: &axum::http::HeaderMap) -> Result<()> {
    if headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .is_some_and(|(scheme, token)| {
            scheme.eq_ignore_ascii_case("bearer")
                && crate::local_app::account::is_placeholder_token(token.trim())
        })
    {
        return Err(crate::Error::Config(
            "本地占位凭据不能发送到 Cursor 官方服务；请在 Cursor 正常登录".into(),
        ));
    }
    Ok(())
}

fn upstream_url(headers: &axum::http::HeaderMap, fallback: &str, path: &str) -> Result<String> {
    let Some(value) = headers.get(UPSTREAM_URL_HEADER) else {
        return Ok(format!("{fallback}{path}"));
    };
    let value = value
        .to_str()
        .map_err(|error| crate::Error::Protocol(format!("invalid upstream URL header: {error}")))?;
    let url = reqwest::Url::parse(value)
        .map_err(|error| crate::Error::Protocol(format!("invalid upstream URL: {error}")))?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https" || !crate::local_app::proxy_host_allowed(host) {
        return Err(crate::Error::Protocol(
            "upstream URL must target a Cursor HTTPS host".into(),
        ));
    }
    Ok(url.into())
}

fn remove_hop_by_hop_headers(headers: &mut axum::http::HeaderMap) {
    let connection_headers = headers
        .get(header::CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for name in connection_headers {
        headers.remove(name);
    }
    for name in [
        header::CONNECTION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn official_free_pro_401_and_5xx_preserve_status_headers_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect(&format!(
            "sqlite://{}",
            dir.path().join("test.db").display()
        ))
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = format!("http://{}", listener.local_addr().unwrap());
        let router = axum::Router::new().fallback(
            |headers: axum::http::HeaderMap, uri: axum::http::Uri| async move {
                assert!(!headers.contains_key(crate::control::auth::TOKEN_HEADER));
                assert_eq!(
                    headers["authorization"],
                    "Bearer synthetic-official-session"
                );
                let (status, body) = match uri.path() {
                    "/free" => (200, "{\"membership\":\"free\"}"),
                    "/pro" => (200, "{\"membership\":\"pro\"}"),
                    "/401" => (401, "unauthorized"),
                    _ => (503, "upstream unavailable"),
                };
                Response::builder()
                    .status(status)
                    .header("x-official-fixture", "unchanged")
                    .body(Body::from(body))
                    .unwrap()
            },
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let proxy = CursorProxy {
            clients: crate::network::NetworkClients::new(store.clone()),
            upstream,
        };
        for (path, status, body) in [
            ("/free", 200, "{\"membership\":\"free\"}"),
            ("/pro", 200, "{\"membership\":\"pro\"}"),
            ("/401", 401, "unauthorized"),
            ("/503", 503, "upstream unavailable"),
        ] {
            let request = Request::builder()
                .uri(path)
                .header("authorization", "Bearer synthetic-official-session")
                .header(
                    crate::control::auth::TOKEN_HEADER,
                    "synthetic-control-token",
                )
                .body(Body::empty())
                .unwrap();
            let response = forward(Extension(proxy.clone()), request).await.unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["x-official-fixture"], "unchanged");
            assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap(), body);
        }
        task.abort();
        store.pool().close().await;
    }
}
