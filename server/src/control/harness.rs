//! Implements local application control endpoints.
use axum::{extract::State, Json};

use crate::{
    local_app::{CursorHarnessStatus, SetEnabled},
    Result,
};

use super::ControlService;

pub async fn status(State(service): State<ControlService>) -> Result<Json<CursorHarnessStatus>> {
    Ok(Json(service.cursor_harness().status().await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionConsent {
    enabled: bool,
    consent: bool,
}

pub async fn subscription(
    State(service): State<ControlService>,
    Json(input): Json<SubscriptionConsent>,
) -> Result<Json<CursorHarnessStatus>> {
    Ok(Json(
        service
            .cursor_harness()
            .set_subscription(input.enabled, input.consent)
            .await?,
    ))
}

pub async fn initialize_ca(
    State(service): State<ControlService>,
) -> Result<Json<CursorHarnessStatus>> {
    Ok(Json(service.cursor_harness().initialize_ca().await?))
}

pub async fn set_enabled(
    State(service): State<ControlService>,
    Json(input): Json<SetEnabled>,
) -> Result<Json<CursorHarnessStatus>> {
    Ok(Json(
        service.cursor_harness().set_enabled(input.enabled).await?,
    ))
}

pub async fn recover(State(service): State<ControlService>) -> Result<Json<CursorHarnessStatus>> {
    service.cursor_harness().recover_pending().await?;
    Ok(Json(service.cursor_harness().status().await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertificateConsent {
    accepted: bool,
    version: u32,
}

pub async fn consent(
    State(service): State<ControlService>,
    Json(input): Json<CertificateConsent>,
) -> Result<Json<CursorHarnessStatus>> {
    Ok(Json(
        service
            .cursor_harness()
            .accept_certificate(input.accepted, input.version)
            .await?,
    ))
}
pub async fn uninstall_certificate(
    State(service): State<ControlService>,
) -> Result<Json<CursorHarnessStatus>> {
    Ok(Json(
        service.cursor_harness().uninstall_certificate().await?,
    ))
}
