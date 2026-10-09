//! Per-process control capability; never sent to Cursor or provider requests.
use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use parking_lot::RwLock;
use std::{net::SocketAddr, sync::Arc};
pub const TOKEN_HEADER: &str = "x-sub2api-control-token";
#[derive(Clone)]
pub struct ControlAuth {
    token: Arc<String>,
    address: Arc<RwLock<Option<SocketAddr>>>,
}
impl Default for ControlAuth {
    fn default() -> Self {
        Self::new()
    }
}
impl ControlAuth {
    pub fn new() -> Self {
        Self {
            token: Arc::new(format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )),
            address: Arc::new(RwLock::new(None)),
        }
    }
    pub fn token(&self) -> &str {
        &self.token
    }
    pub fn bind(&self, address: SocketAddr) {
        *self.address.write() = Some(address);
    }
    fn authorized(&self, request: &Request<Body>) -> bool {
        let Some(address) = *self.address.read() else {
            return false;
        };
        let expected = format!("127.0.0.1:{}", address.port());
        let headers = request.headers();
        if headers.get_all(header::HOST).iter().count() != 1
            || headers.get(header::HOST).and_then(|v| v.to_str().ok()) != Some(expected.as_str())
        {
            return false;
        }
        if headers.get_all(TOKEN_HEADER).iter().count() != 1 {
            return false;
        }
        let supplied = headers
            .get(TOKEN_HEADER)
            .map(|v| v.as_bytes())
            .unwrap_or_default();
        let wanted = self.token.as_bytes();
        if supplied.len() != wanted.len()
            || supplied
                .iter()
                .zip(wanted)
                .fold(0u8, |diff, (a, b)| diff | (a ^ b))
                != 0
        {
            return false;
        }
        if headers.get_all(header::ORIGIN).iter().count() > 1 {
            return false;
        }
        if let Some(origin) = headers.get(header::ORIGIN) {
            if origin.to_str().ok() != Some(format!("http://{expected}").as_str()) {
                return false;
            }
        }
        true
    }
}
pub async fn protect(
    State(auth): State<ControlAuth>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    if request.uri().path().starts_with("/byok/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.uri().path().starts_with("/__byok-api__/api/") && !auth.authorized(&request) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"code":"forbidden", "message":"控制请求未通过鉴权"})),
        )
            .into_response();
    }
    request.headers_mut().remove(TOKEN_HEADER);
    next.run(request).await
}
