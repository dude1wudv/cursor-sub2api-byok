//! Network preferences never alter Windows proxy settings or a live takeover.
use super::ControlService;
use crate::{
    network::NetworkClients,
    store::{PortSettings, ProxySettings, ProxySettingsInput},
    Error, Result,
};
use axum::{
    extract::{Extension, State},
    Json,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct Settings {
    ports: PortSettings,
    outbound: ProxySettings,
    actual_service_port: Option<u16>,
    actual_proxy_port: Option<u16>,
}
pub async fn get(State(s): State<ControlService>) -> Result<Json<Settings>> {
    Ok(Json(Settings {
        ports: s.store.port_settings().await?,
        outbound: s.store.proxy_settings().await?,
        actual_service_port: s.cursor_harness().backend_addr().map(|a| a.port()),
        actual_proxy_port: s.cursor_harness().proxy_port().await,
    }))
}
pub async fn ports(
    State(s): State<ControlService>,
    Json(p): Json<PortSettings>,
) -> Result<Json<Settings>> {
    let _guard = s.cursor_harness().configuration_guard().await?;
    if p.proxy_port != 0 && p.proxy_port == p.service_port {
        return Err(Error::Config("接管端口和管理端口不能相同".into()));
    }
    let outbound = s.store.proxy_settings().await?;
    if outbound.mode.is_custom() {
        crate::network::reject_self_proxy(&outbound.address, p.proxy_port)?;
        crate::network::reject_self_proxy(&outbound.address, p.service_port)?;
    }
    let current = s.cursor_harness().backend_addr().map(|a| a.port());
    let mut reservations = Vec::new();
    for port in [p.proxy_port, p.service_port] {
        if port != 0 && Some(port) != current {
            reservations.push(
                tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
                    .await
                    .map_err(|_| Error::Config(format!("端口 {port} 已被占用或不可用")))?,
            );
        }
    }
    if Some(p.proxy_port) == current {
        return Err(Error::Config("接管端口不能使用当前管理端口".into()));
    }
    s.store.set_port_settings(p).await?;
    get(State(s.clone())).await
}
pub async fn proxy(
    State(s): State<ControlService>,
    Extension(clients): Extension<NetworkClients>,
    Json(input): Json<ProxySettingsInput>,
) -> Result<Json<Settings>> {
    let _guard = s.cursor_harness().configuration_guard().await?;
    if input.mode.is_custom() {
        let ports = s.store.port_settings().await?;
        crate::network::reject_self_proxy(&input.address, ports.proxy_port)?;
        crate::network::reject_self_proxy(&input.address, ports.service_port)?;
        if let Some(a) = s.cursor_harness().backend_addr() {
            crate::network::reject_self_proxy(&input.address, a.port())?;
        }
    }
    s.store.set_proxy_settings(input).await?;
    clients.invalidate().await;
    get(State(s.clone())).await
}
#[derive(Serialize)]
pub struct TestResult {
    status: u16,
    duration_ms: u64,
    target: &'static str,
}
pub async fn test(State(s): State<ControlService>) -> Result<Json<TestResult>> {
    // Public health request, no account token or model usage; redirects disabled.
    let started = std::time::Instant::now();
    let client = crate::network::client_builder(&s.store)
        .await?
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let r = client
        .get("https://api2.cursor.sh/")
        .send()
        .await
        .map_err(|_| Error::Config("出站连接失败；未改为其他代理或直连重试".into()))?;
    Ok(Json(TestResult {
        status: r.status().as_u16(),
        duration_ms: started.elapsed().as_millis() as u64,
        target: "https://api2.cursor.sh/",
    }))
}
