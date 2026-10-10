//! Read-only CC Switch price import. Never reads account configuration or writes the source.
use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::io::AsyncReadExt;

use super::{now_ms, ModelPricingSettings, ModelTokenPrice, Store, SETTINGS_KEY};
use crate::{Error, Result};

const SYNC_KEY: &str = "cc_switch_pricing_sync";
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PricingSyncSettings {
    pub enabled: bool,
    pub source_path: String,
}

impl Default for PricingSyncSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            source_path: dirs::home_dir()
                .unwrap_or_default()
                .join(".cc-switch/model-pricing.json")
                .to_string_lossy()
                .into_owned(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PricingSyncStatus {
    pub settings: PricingSyncSettings,
    pub checked_at_ms: Option<i64>,
    pub synced_at_ms: Option<i64>,
    pub stale: bool,
    pub error: Option<String>,
    pub matched: BTreeMap<String, String>,
    pub unmatched: Vec<String>,
    pub ambiguous: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Source {
    version: u32,
    models: Vec<SourceModel>,
    #[serde(default)]
    deleted_model_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceModel {
    model_id: String,
    input_cost_per_million: String,
    output_cost_per_million: String,
    cache_read_cost_per_million: String,
    cache_creation_cost_per_million: String,
}

fn parse_prices(
    bytes: &[u8],
) -> std::result::Result<BTreeMap<String, ModelTokenPrice>, &'static str> {
    let source: Source = serde_json::from_slice(bytes)
        .map_err(|_| "价格文件不是完整有效的 CC Switch JSON，保留上次有效单价")?;
    if source.version != 1 || source.models.len() > 20_000 {
        return Err("不支持的 CC Switch 价格文件版本或模型数量，保留上次有效单价");
    }
    let mut prices = BTreeMap::new();
    for model in source.models {
        if model.model_id.is_empty() || model.model_id.len() > 256 {
            return Err("价格文件包含无效模型 ID，保留上次有效单价");
        }
        let values = [
            model.input_cost_per_million,
            model.output_cost_per_million,
            model.cache_read_cost_per_million,
            model.cache_creation_cost_per_million,
        ]
        .map(|value| value.parse::<f64>());
        let [Ok(input), Ok(output), Ok(read), Ok(write)] = values else {
            return Err("价格文件包含无效价格，保留上次有效单价");
        };
        if [input, output, read, write]
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1_000_000_000.0).contains(v))
        {
            return Err("价格必须是有限的非负金额，保留上次有效单价");
        }
        let id = model.model_id;
        if prices
            .insert(
                id.clone(),
                ModelTokenPrice {
                    input_per_million: input,
                    output_per_million: output,
                    cache_read_per_million: read,
                    cache_write_per_million: write,
                },
            )
            .is_some()
        {
            return Err("价格文件包含重复模型 ID，保留上次有效单价");
        }
    }
    for deleted in source.deleted_model_ids {
        prices.remove(&deleted);
    }
    Ok(prices)
}

fn bare_id(id: &str) -> &str {
    id.split_once('/').map_or(id, |(_, suffix)| suffix)
}

fn match_price<'a>(
    id: &str,
    prices: &'a BTreeMap<String, ModelTokenPrice>,
) -> std::result::Result<Option<(&'a String, &'a ModelTokenPrice)>, ()> {
    if let Some(exact) = prices.get_key_value(id) {
        return Ok(Some(exact));
    }
    let mut candidates = prices.iter().filter(|(key, _)| bare_id(key) == bare_id(id));
    let result = candidates.next();
    if candidates.next().is_some() {
        Err(())
    } else {
        Ok(result)
    }
}

impl Store {
    pub async fn pricing_sync_status(&self) -> Result<PricingSyncStatus> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM service_settings WHERE setting_key = ?")
                .bind(SYNC_KEY)
                .fetch_optional(&self.pool)
                .await?;
        Ok(value
            .map(|v| serde_json::from_str(&v))
            .transpose()?
            .unwrap_or_default())
    }

    pub async fn set_pricing_sync(
        &self,
        settings: PricingSyncSettings,
    ) -> Result<PricingSyncStatus> {
        let path = PathBuf::from(&settings.source_path);
        if settings.source_path.len() > 4096
            || !path.is_absolute()
            || path.file_name().and_then(|name| name.to_str()) != Some("model-pricing.json")
        {
            return Err(Error::Config(
                "请选择绝对路径的 model-pricing.json 文件".into(),
            ));
        }
        let _write = self.writes.lock().await;
        let mut state = self.pricing_sync_status().await?;
        if state.settings.source_path != settings.source_path {
            state.matched.clear();
            state.synced_at_ms = None;
        }
        state.settings = settings;
        state.error = None;
        state.stale = state.settings.enabled;
        self.save_pricing_sync(&state).await?;
        Ok(state)
    }

    async fn save_pricing_sync(&self, state: &PricingSyncStatus) -> Result<()> {
        sqlx::query("INSERT INTO service_settings(setting_key, value_json, updated_at_ms) VALUES (?, ?, ?) ON CONFLICT(setting_key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms")
            .bind(SYNC_KEY).bind(serde_json::to_string(state)?).bind(now_ms()).execute(&self.pool).await?;
        Ok(())
    }

    /// Called before control-plane estimates and price reads; shares the settings write lock.
    pub async fn refresh_pricing_sync(&self) -> Result<PricingSyncStatus> {
        let _write = self.writes.lock().await;
        let mut state = self.pricing_sync_status().await?;
        if !state.settings.enabled {
            return Ok(state);
        }
        state.checked_at_ms = Some(now_ms());
        let source = async {
            let file = tokio::fs::File::open(&state.settings.source_path)
                .await
                .map_err(|_| "无法读取 CC Switch 价格文件，保留上次有效单价")?;
            let mut bytes = Vec::new();
            file.take(MAX_SOURCE_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "价格文件读取失败，保留上次有效单价")?;
            if bytes.len() as u64 > MAX_SOURCE_BYTES {
                return Err("价格文件超过 8 MiB，保留上次有效单价");
            }
            parse_prices(&bytes)
        }
        .await;
        let prices = match source {
            Ok(prices) => prices,
            Err(error) => {
                state.stale = true;
                state.error = Some(error.into());
                self.save_pricing_sync(&state).await?;
                return Ok(state);
            }
        };
        let mut settings = self.model_pricing().await?;
        if settings.currency != "USD" {
            state.stale = true;
            state.error = Some(
                "同步价格为 USD；请关闭同步并将本地单价配置为 USD 后再启用，避免混合币种".into(),
            );
            self.save_pricing_sync(&state).await?;
            return Ok(state);
        }
        // Only read model identity columns, never credentials.
        let models = sqlx::query("SELECT model_hash, model_id FROM model_configs")
            .fetch_all(&self.pool)
            .await?;
        state.matched.clear();
        state.unmatched.clear();
        state.ambiguous.clear();
        for model in models {
            let hash: String = model.try_get("model_hash")?;
            let id: String = model.try_get("model_id")?;
            match match_price(&id, &prices) {
                Ok(Some((source_id, price))) => {
                    settings.models.insert(hash.clone(), price.clone());
                    state.matched.insert(hash, source_id.clone());
                }
                Ok(None) => state.unmatched.push(id),
                Err(()) => state.ambiguous.push(id),
            }
        }
        settings.validate()?;
        state.stale = !state.unmatched.is_empty() || !state.ambiguous.is_empty();
        state.error = None;
        state.synced_at_ms = Some(now_ms());
        let mut tx = self.pool.begin().await?;
        for (key, value) in [
            (SETTINGS_KEY, serde_json::to_string(&settings)?),
            (SYNC_KEY, serde_json::to_string(&state)?),
        ] {
            sqlx::query("INSERT INTO service_settings(setting_key, value_json, updated_at_ms) VALUES (?, ?, ?) ON CONFLICT(setting_key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms")
                .bind(key).bind(value).bind(now_ms()).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(state)
    }

    pub async fn set_manual_model_pricing(
        &self,
        settings: ModelPricingSettings,
    ) -> Result<ModelPricingSettings> {
        settings.validate()?;
        let _write = self.writes.lock().await;
        if self.pricing_sync_status().await?.settings.enabled {
            return Err(Error::Config(
                "自动同步已开启，请先关闭同步再手动修改单价".into(),
            ));
        }
        sqlx::query("INSERT INTO service_settings(setting_key, value_json, updated_at_ms) VALUES (?, ?, ?) ON CONFLICT(setting_key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms")
            .bind(SETTINGS_KEY).bind(serde_json::to_string(&settings)?).bind(now_ms()).execute(&self.pool).await?;
        Ok(settings)
    }
}
