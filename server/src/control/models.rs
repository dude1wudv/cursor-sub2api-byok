//! Implements model configuration endpoints.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{model::ModelConfig, store::Sub2ApiModelInput, Result};

use super::{ControlService, ModelConnectivityResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveModels {
    pub models: Vec<Sub2ApiModelInput>,
}

#[derive(Deserialize)]
pub struct ModelOrder {
    pub model_hashes: Vec<String>,
}

pub async fn list(State(service): State<ControlService>) -> Result<Json<Vec<ModelConfig>>> {
    Ok(Json(service.models().await?))
}

pub async fn create(
    State(service): State<ControlService>,
    input: std::result::Result<Json<SaveModels>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<Vec<ModelConfig>>)> {
    let Json(input) =
        input.map_err(|_| crate::Error::Config("invalid Sub2API model input".into()))?;
    Ok((
        StatusCode::CREATED,
        Json(service.create_models(&input.models).await?),
    ))
}

pub async fn reorder(
    State(service): State<ControlService>,
    Json(input): Json<ModelOrder>,
) -> Result<Json<Vec<ModelConfig>>> {
    Ok(Json(service.reorder_models(&input.model_hashes).await?))
}

pub async fn remove(
    State(service): State<ControlService>,
    Path(model_hash): Path<String>,
) -> Result<StatusCode> {
    service.delete_model(&model_hash).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update(
    State(service): State<ControlService>,
    Path(model_hash): Path<String>,
    input: std::result::Result<Json<Sub2ApiModelInput>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<ModelConfig>> {
    let Json(input) =
        input.map_err(|_| crate::Error::Config("invalid Sub2API model input".into()))?;
    Ok(Json(service.update_model(&model_hash, &input).await?))
}

pub async fn test(
    State(service): State<ControlService>,
    Path((model_hash, test_id)): Path<(String, String)>,
) -> Result<Json<ModelConnectivityResult>> {
    Ok(Json(service.test_model(&model_hash, &test_id).await?))
}

pub async fn cancel(
    State(service): State<ControlService>,
    Path((_model_hash, test_id)): Path<(String, String)>,
) -> Result<StatusCode> {
    service.cancel_model_test(&test_id);
    Ok(StatusCode::NO_CONTENT)
}
