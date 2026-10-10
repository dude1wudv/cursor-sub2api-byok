//! Prices are local preferences; no upstream billing or account data is changed.
use super::{overview::OverviewRange, ControlService};
use crate::{
    store::{CostEstimate, ModelPricingSettings, PricingSyncSettings, PricingSyncStatus},
    Result,
};
use axum::{
    extract::{Query, State},
    Json,
};

pub async fn get(State(service): State<ControlService>) -> Result<Json<ModelPricingSettings>> {
    service.store.refresh_pricing_sync().await?;
    Ok(Json(service.store.model_pricing().await?))
}

pub async fn put(
    State(service): State<ControlService>,
    Json(settings): Json<ModelPricingSettings>,
) -> Result<Json<ModelPricingSettings>> {
    Ok(Json(
        service.store.set_manual_model_pricing(settings).await?,
    ))
}

pub async fn get_sync(State(service): State<ControlService>) -> Result<Json<PricingSyncStatus>> {
    Ok(Json(service.store.pricing_sync_status().await?))
}

pub async fn put_sync(
    State(service): State<ControlService>,
    Json(settings): Json<PricingSyncSettings>,
) -> Result<Json<PricingSyncStatus>> {
    service.store.set_pricing_sync(settings).await?;
    Ok(Json(service.store.refresh_pricing_sync().await?))
}

pub async fn estimate(
    State(service): State<ControlService>,
    Query(range): Query<OverviewRange>,
) -> Result<Json<CostEstimate>> {
    service.store.refresh_pricing_sync().await?;
    Ok(Json(
        service
            .store
            .cost_estimate(range.start_ms, range.end_ms, range.model_hashes.as_deref())
            .await?,
    ))
}
