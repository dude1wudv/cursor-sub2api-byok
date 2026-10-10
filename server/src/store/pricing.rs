//! User configured prices and local estimates over recorded provider usage.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::Row;

use super::{
    now_ms,
    overview::{fresh_input_sql, validate_range},
    Store,
};
use crate::{Error, Result};

const SETTINGS_KEY: &str = "model_token_pricing";

#[path = "pricing_sync.rs"]
mod sync;
pub use sync::{PricingSyncSettings, PricingSyncStatus};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelTokenPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
    pub cache_read_per_million: f64,
    pub cache_write_per_million: f64,
}

impl ModelTokenPrice {
    fn values(&self) -> [f64; 4] {
        [
            self.input_per_million,
            self.output_per_million,
            self.cache_read_per_million,
            self.cache_write_per_million,
        ]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelPricingSettings {
    pub currency: String,
    pub models: BTreeMap<String, ModelTokenPrice>,
}

impl Default for ModelPricingSettings {
    fn default() -> Self {
        Self {
            currency: "USD".into(),
            models: BTreeMap::new(),
        }
    }
}

impl ModelPricingSettings {
    fn validate(&self) -> Result<()> {
        if self.currency.len() != 3 || !self.currency.bytes().all(|byte| byte.is_ascii_uppercase())
        {
            return Err(Error::Config(
                "currency must be a three-letter uppercase currency code".into(),
            ));
        }
        if self.models.len() > 500
            || self.models.iter().any(|(key, price)| {
                key.is_empty()
                    || key.len() > 256
                    || price
                        .values()
                        .iter()
                        .any(|value| !value.is_finite() || !(0.0..=1_000_000_000.0).contains(value))
            })
        {
            return Err(Error::Config(
                "prices must be finite, nonnegative amounts per million tokens".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ModelCostEstimate {
    pub model_hash: Option<String>,
    pub display_name: String,
    pub calls: i64,
    pub unknown_usage_calls: i64,
    pub recorded_tokens: i64,
    pub amount: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct CostEstimate {
    pub currency: String,
    pub amount: f64,
    pub covered_tokens: i64,
    pub unpriced_tokens: i64,
    pub unknown_usage_calls: i64,
    pub models: Vec<ModelCostEstimate>,
}

impl Store {
    pub async fn model_pricing(&self) -> Result<ModelPricingSettings> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM service_settings WHERE setting_key = ?")
                .bind(SETTINGS_KEY)
                .fetch_optional(&self.pool)
                .await?;
        let settings: ModelPricingSettings = value
            .map(|value| serde_json::from_str(&value))
            .transpose()?
            .unwrap_or_default();
        settings.validate()?;
        Ok(settings)
    }

    pub async fn set_model_pricing(
        &self,
        settings: ModelPricingSettings,
    ) -> Result<ModelPricingSettings> {
        settings.validate()?;
        let _write = self.writes.lock().await;
        sqlx::query("INSERT INTO service_settings(setting_key, value_json, updated_at_ms) VALUES (?, ?, ?) ON CONFLICT(setting_key) DO UPDATE SET value_json = excluded.value_json, updated_at_ms = excluded.updated_at_ms")
            .bind(SETTINGS_KEY).bind(serde_json::to_string(&settings)?).bind(now_ms()).execute(&self.pool).await?;
        Ok(settings)
    }

    pub async fn cost_estimate(
        &self,
        start: Option<i64>,
        end: Option<i64>,
        models: Option<&str>,
    ) -> Result<CostEstimate> {
        validate_range(start, end, models, None)?;
        let settings = self.model_pricing().await?;
        let rows = sqlx::query(&format!(
            "SELECT model_hash, MAX(display_name) AS display_name, COUNT(*) AS calls,
                COALESCE(SUM(input_tokens IS NULL OR output_tokens IS NULL), 0) AS unknown_usage_calls,
                COALESCE(SUM({fresh}), 0) AS input_tokens,
                COALESCE(SUM(MAX(0, COALESCE(output_tokens, 0))), 0) AS output_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_read_tokens, 0))), 0) AS cache_read_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_write_tokens, 0))), 0) AS cache_write_tokens
             FROM llm_calls WHERE (? IS NULL OR created_at_ms >= ?) AND (? IS NULL OR created_at_ms < ?)
               AND (? IS NULL OR model_hash IN (SELECT value FROM json_each(?)))
             GROUP BY model_hash ORDER BY display_name", fresh = fresh_input_sql()))
            .bind(start).bind(start).bind(end).bind(end).bind(models).bind(models).fetch_all(&self.pool).await?;
        let mut estimate = CostEstimate {
            currency: settings.currency,
            amount: 0.0,
            covered_tokens: 0,
            unpriced_tokens: 0,
            unknown_usage_calls: 0,
            models: Vec::new(),
        };
        for row in rows {
            let model_hash: Option<String> = row.try_get("model_hash")?;
            let tokens: [i64; 4] = [
                row.try_get("input_tokens")?,
                row.try_get("output_tokens")?,
                row.try_get("cache_read_tokens")?,
                row.try_get("cache_write_tokens")?,
            ];
            let recorded_tokens = tokens
                .iter()
                .fold(0_i64, |total, value| total.saturating_add(*value));
            let amount = model_hash
                .as_ref()
                .and_then(|key| settings.models.get(key))
                .map(|price| estimate_tokens(tokens, price));
            if let Some(amount) = amount {
                estimate.amount += amount;
                estimate.covered_tokens = estimate.covered_tokens.saturating_add(recorded_tokens);
            } else {
                estimate.unpriced_tokens = estimate.unpriced_tokens.saturating_add(recorded_tokens);
            }
            let unknown_usage_calls: i64 = row.try_get("unknown_usage_calls")?;
            estimate.unknown_usage_calls += unknown_usage_calls;
            estimate.models.push(ModelCostEstimate {
                model_hash,
                display_name: row.try_get("display_name")?,
                calls: row.try_get("calls")?,
                recorded_tokens,
                unknown_usage_calls,
                amount,
            });
        }
        Ok(estimate)
    }
}

fn estimate_tokens(tokens: [i64; 4], price: &ModelTokenPrice) -> f64 {
    tokens
        .into_iter()
        .zip(price.values())
        .map(|(tokens, price)| tokens as f64 / 1_000_000.0 * price)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinct_categories_are_charged_once() {
        let price = ModelTokenPrice {
            input_per_million: 2.0,
            output_per_million: 8.0,
            cache_read_per_million: 0.2,
            cache_write_per_million: 2.5,
        };
        assert_eq!(
            estimate_tokens([1_000_000, 2_000_000, 3_000_000, 4_000_000], &price),
            28.6
        );
    }
    #[test]
    fn prices_reject_negative_nonfinite_and_unknown_currency_shape() {
        let mut settings = ModelPricingSettings::default();
        assert!(settings.validate().is_ok());
        settings.models.insert(
            "model".into(),
            ModelTokenPrice {
                input_per_million: f64::NAN,
                ..Default::default()
            },
        );
        assert!(settings.validate().is_err());
        settings.models.get_mut("model").unwrap().input_per_million = -1.0;
        assert!(settings.validate().is_err());
        settings.models.clear();
        settings.currency = "$".into();
        assert!(settings.validate().is_err());
    }
}
