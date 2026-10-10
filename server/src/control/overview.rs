//! Implements control dashboard overview endpoints.
//! HTTP handler for the desktop overview aggregates.

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::{
    model::{Overview, TokenUsageGranularity},
    store::UsageCalendarDay,
    Result,
};

use super::ControlService;

#[derive(Debug, Default, Deserialize)]
pub struct OverviewRange {
    pub(super) start_ms: Option<i64>,
    pub(super) end_ms: Option<i64>,
    pub(super) model_hashes: Option<String>,
    bucket_ms: Option<i64>,
    timezone: Option<String>,
}

#[derive(Serialize)]
pub struct UsageOverview {
    #[serde(flatten)]
    overview: Overview,
    calendar: Vec<UsageCalendarDay>,
    timezone: String,
}

pub async fn get(
    State(service): State<ControlService>,
    Query(range): Query<OverviewRange>,
) -> Result<Json<UsageOverview>> {
    let mut overview = service
        .overview(
            range.start_ms,
            range.end_ms,
            range.model_hashes.as_deref(),
            range.bucket_ms,
        )
        .await?;
    let timezone = range.timezone.unwrap_or_else(|| "UTC".into());
    let end = range
        .end_ms
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let start = range.start_ms.unwrap_or(end - 365 * 86_400_000);
    let calendar = service
        .store
        .usage_calendar(start, end, range.model_hashes.as_deref(), &timezone)
        .await?;
    if overview.token_usage_granularity == TokenUsageGranularity::Day && range.bucket_ms.is_none() {
        overview.token_usage_series = calendar.iter().map(|day| day.usage.clone()).collect();
    }
    Ok(Json(UsageOverview {
        overview,
        calendar,
        timezone,
    }))
}
