//! Fetch only model IDs from the saved Sub2API connection. Never expose credentials.
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};
use crate::{Error, Result};
use super::ControlService;

#[derive(Deserialize)]
struct ModelList { data: Vec<ModelId> }
#[derive(Deserialize, Serialize)]
pub(super) struct ModelId { id: String }

pub(super) async fn list(State(service): State<ControlService>) -> Result<Json<Vec<ModelId>>> {
    let (base, key) = service.store.sub2api_credentials().await?;
    let url = format!("{}{}/models", base, if base.ends_with("/v1") { "" } else { "/v1" });
    let client = crate::network::client_builder(&service.store).await?
        .timeout(Duration::from_secs(20)).redirect(reqwest::redirect::Policy::none()).build()?;
    let mut response = client.get(url).bearer_auth(key).send().await
        .map_err(|_| Error::Config("获取模型列表失败，请检查连接或网络".into()))?;
    if !response.status().is_success() {
        return Err(Error::Config(format!("获取模型列表失败（HTTP {}）", response.status().as_u16())));
    }
    const MAX_BYTES: usize = 2 * 1024 * 1024;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::Config("读取模型列表失败".into()))? {
        if body.len() + chunk.len() > MAX_BYTES { return Err(Error::Config("模型列表超过 2 MiB 限制".into())); }
        body.extend_from_slice(&chunk);
    }
    let list: ModelList = serde_json::from_slice(&body).map_err(|_| Error::Config("模型列表格式无效，需要 data 数组及模型 id".into()))?;
    let ids: BTreeSet<_> = list.data.into_iter().map(|m| m.id).filter(|id| !id.trim().is_empty()).collect();
    if ids.len() > 2000 { return Err(Error::Config("模型列表超过 2000 个模型限制".into())); }
    Ok(Json(ids.into_iter().map(|id| ModelId { id }).collect()))
}
