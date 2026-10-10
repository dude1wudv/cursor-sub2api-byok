//! Prices are local preferences; no upstream billing or account data is changed.
use super::{overview::OverviewRange, ControlService};
use crate::{
    store::{CostEstimate, ModelPricingSettings},
    Result,
};
use axum::{
    extract::{Query, State},
    Json,
};

pub async fn get(State(service): State<ControlService>) -> Result<Json<ModelPricingSettings>> {
    Ok(Json(service.store.model_pricing().await?))
}

pub async fn put(
    State(service): State<ControlService>,
    Json(settings): Json<ModelPricingSettings>,
) -> Result<Json<ModelPricingSettings>> {
    Ok(Json(service.store.set_model_pricing(settings).await?))
}

pub async fn estimate(
    State(service): State<ControlService>,
    Query(range): Query<OverviewRange>,
) -> Result<Json<CostEstimate>> {
    Ok(Json(
        service
            .store
            .cost_estimate(range.start_ms, range.end_ms, range.model_hashes.as_deref())
            .await?,
    ))
}
