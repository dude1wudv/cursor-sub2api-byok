//! Provides aggregate data for the control API.
//! Efficient database aggregates for the desktop overview.

use std::collections::BTreeMap;

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use sqlx::Row;

use crate::{
    model::{Overview, OverviewMetrics, TokenUsageBucket, TokenUsageGranularity},
    Error, Result,
};

use super::Store;

const OVERVIEW_DAYS: u64 = 365;
const MAX_RANGE_BUCKETS: i64 = 60;
const MAX_EXPLICIT_BUCKETS: i64 = 16_000;
const MAX_RANGE_MS: i64 = 3660 * DAY_MS;
const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

impl Store {
    pub async fn overview(
        &self,
        start_ms: Option<i64>,
        end_ms: Option<i64>,
        model_hashes: Option<&str>,
        bucket_ms: Option<i64>,
    ) -> Result<Overview> {
        validate_range(start_ms, end_ms, model_hashes, bucket_ms)?;
        let call_row = sqlx::query(
            "SELECT
                COUNT(*) AS llm_calls,
                COALESCE(SUM(status = 'completed'), 0) AS successful_calls,
                COALESCE(SUM(status != 'completed'), 0) AS failed_calls
             FROM llm_calls
             WHERE status != 'running'
               AND (? IS NULL OR created_at_ms >= ?)
               AND (? IS NULL OR created_at_ms < ?)
               AND (? IS NULL OR model_hash IN (SELECT value FROM json_each(?)))",
        )
        .bind(start_ms)
        .bind(start_ms)
        .bind(end_ms)
        .bind(end_ms)
        .bind(model_hashes)
        .bind(model_hashes)
        .fetch_one(&self.pool)
        .await?;
        let token_row = sqlx::query(&format!(
            "SELECT
                COALESCE(SUM({fresh_input}), 0) AS input_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_read_tokens, 0))), 0) AS cache_read_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_write_tokens, 0))), 0) AS cache_write_tokens,
                COALESCE(SUM(MAX(0, COALESCE(output_tokens, 0))), 0) AS output_tokens
             FROM llm_calls
             WHERE (? IS NULL OR created_at_ms >= ?)
               AND (? IS NULL OR created_at_ms < ?)
               AND (? IS NULL OR model_hash IN (SELECT value FROM json_each(?)))",
            fresh_input = fresh_input_sql(),
        ))
        .bind(start_ms)
        .bind(start_ms)
        .bind(end_ms)
        .bind(end_ms)
        .bind(model_hashes)
        .bind(model_hashes)
        .fetch_one(&self.pool)
        .await?;

        let input_tokens = non_negative(token_row.try_get("input_tokens")?);
        let cache_read_tokens = non_negative(token_row.try_get("cache_read_tokens")?);
        let cache_write_tokens = non_negative(token_row.try_get("cache_write_tokens")?);
        let output_tokens = non_negative(token_row.try_get("output_tokens")?);
        let prompt_tokens = saturating_sum(&[input_tokens, cache_read_tokens, cache_write_tokens]);
        let metrics = OverviewMetrics {
            llm_calls: call_row.try_get("llm_calls")?,
            successful_calls: call_row.try_get("successful_calls")?,
            failed_calls: call_row.try_get("failed_calls")?,
            token_usage: prompt_tokens.saturating_add(output_tokens),
            prompt_tokens,
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            output_tokens,
        };

        let (token_usage_granularity, bucket_ms, series_start_ms, bucket_count) =
            token_usage_buckets(start_ms, end_ms, bucket_ms)?;
        let rows = sqlx::query(&format!(
            "SELECT
                (created_at_ms / {bucket_ms}) * {bucket_ms} AS bucket_start_ms,
                COALESCE(SUM({fresh_input}), 0) AS input_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_read_tokens, 0))), 0) AS cache_read_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_write_tokens, 0))), 0) AS cache_write_tokens,
                COALESCE(SUM(MAX(0, COALESCE(output_tokens, 0))), 0) AS output_tokens
             FROM llm_calls
             WHERE created_at_ms >= ?
               AND (? IS NULL OR created_at_ms < ?)
               AND (? IS NULL OR model_hash IN (SELECT value FROM json_each(?)))
             GROUP BY bucket_start_ms
             ORDER BY bucket_start_ms",
            fresh_input = fresh_input_sql(),
        ))
        .bind(start_ms.unwrap_or(series_start_ms).max(series_start_ms))
        .bind(end_ms)
        .bind(end_ms)
        .bind(model_hashes)
        .bind(model_hashes)
        .fetch_all(&self.pool)
        .await?;
        let mut recorded = rows
            .into_iter()
            .map(|row| {
                let bucket_start_ms: i64 = row.try_get("bucket_start_ms")?;
                Ok((
                    bucket_start_ms,
                    TokenUsageBucket {
                        bucket_start_ms,
                        input_tokens: non_negative(row.try_get("input_tokens")?),
                        cache_read_tokens: non_negative(row.try_get("cache_read_tokens")?),
                        cache_write_tokens: non_negative(row.try_get("cache_write_tokens")?),
                        output_tokens: non_negative(row.try_get("output_tokens")?),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let token_usage_series = (0..bucket_count)
            .map(|offset| series_start_ms.saturating_add(offset.saturating_mul(bucket_ms)))
            .map(|bucket_start_ms| {
                recorded
                    .remove(&bucket_start_ms)
                    .unwrap_or(TokenUsageBucket {
                        bucket_start_ms,
                        ..TokenUsageBucket::default()
                    })
            })
            .collect();

        Ok(Overview {
            metrics,
            token_usage_granularity,
            token_usage_series,
        })
    }
}

fn token_usage_buckets(
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    requested_bucket_ms: Option<i64>,
) -> Result<(TokenUsageGranularity, i64, i64, i64)> {
    if let (Some(start_ms), Some(end_ms)) = (start_ms, end_ms) {
        let duration_ms = end_ms.saturating_sub(start_ms).max(1);
        let explicit_bucket_ms = requested_bucket_ms.filter(|bucket_ms| *bucket_ms >= MINUTE_MS);
        let bucket_ms = explicit_bucket_ms.unwrap_or({
            if duration_ms <= HOUR_MS {
                MINUTE_MS
            } else if duration_ms <= MAX_RANGE_BUCKETS * HOUR_MS {
                HOUR_MS
            } else {
                DAY_MS
            }
        });
        let granularity = if bucket_ms < HOUR_MS {
            TokenUsageGranularity::Minute
        } else if bucket_ms < DAY_MS {
            TokenUsageGranularity::Hour
        } else {
            TokenUsageGranularity::Day
        };
        let last_bucket_ms = end_ms.saturating_sub(1).div_euclid(bucket_ms) * bucket_ms;
        let first_bucket_ms = start_ms.div_euclid(bucket_ms) * bucket_ms;
        let bucket_count = (last_bucket_ms - first_bucket_ms).div_euclid(bucket_ms) + 1;
        if bucket_count > MAX_EXPLICIT_BUCKETS {
            return Err(Error::Config(
                "too many usage buckets; choose a larger bucket_ms".into(),
            ));
        }
        let series_start_ms =
            last_bucket_ms.saturating_sub((bucket_count - 1).saturating_mul(bucket_ms));
        return Ok((granularity, bucket_ms, series_start_ms, bucket_count));
    }

    let today_start_ms = Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|value| value.and_utc().timestamp_millis())
        .unwrap_or(0);
    let series_start_ms = today_start_ms.saturating_sub(
        i64::try_from(OVERVIEW_DAYS - 1)
            .unwrap_or(0)
            .saturating_mul(DAY_MS),
    );
    Ok((
        TokenUsageGranularity::Day,
        DAY_MS,
        series_start_ms,
        i64::try_from(OVERVIEW_DAYS).unwrap_or(0),
    ))
}

pub(super) fn fresh_input_sql() -> &'static str {
    "CASE
        WHEN request_type = 'anthropic' THEN MAX(0, COALESCE(input_tokens, 0))
        ELSE MAX(0, COALESCE(input_tokens, 0)
            - COALESCE(cache_read_tokens, 0)
            - COALESCE(cache_write_tokens, 0))
     END"
}

fn non_negative(value: i64) -> i64 {
    value.max(0)
}

fn saturating_sum(values: &[i64]) -> i64 {
    values
        .iter()
        .fold(0_i64, |total, value| total.saturating_add(*value))
}

pub(crate) fn validate_range(
    start: Option<i64>,
    end: Option<i64>,
    models: Option<&str>,
    bucket: Option<i64>,
) -> Result<()> {
    match (start, end) {
        (Some(start), Some(end)) if start >= 0 && end > start && end - start <= MAX_RANGE_MS => {}
        (None, None) => {}
        _ => {
            return Err(Error::Config(
                "usage range must have start_ms < end_ms and span at most 3660 days".into(),
            ))
        }
    }
    if bucket.is_some_and(|value| !(MINUTE_MS..=MAX_RANGE_MS).contains(&value)) {
        return Err(Error::Config(
            "bucket_ms must be at least one minute and at most 3660 days".into(),
        ));
    }
    if let Some(models) = models {
        let values: Vec<String> = serde_json::from_str(models)?;
        if values.len() > 500 || values.iter().any(|value| value.len() > 256) {
            return Err(Error::Config("invalid usage model filter".into()));
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct UsageCalendarDay {
    pub date: String,
    pub end_ms: i64,
    pub calls: i64,
    #[serde(flatten)]
    pub usage: TokenUsageBucket,
}

// Calendar boundaries are computed with IANA rules, never a fixed UTC offset.
// A civil day can contain 23/25 hours or even be skipped by a timezone change.
fn calendar_boundaries(start: i64, end: i64, timezone: Tz) -> Result<Vec<(String, i64, i64)>> {
    let date_at = |value| {
        DateTime::from_timestamp_millis(value)
            .map(|value| value.with_timezone(&timezone).date_naive())
            .ok_or_else(|| Error::Config("usage date is out of range".into()))
    };
    let first = date_at(start)?;
    let last = date_at(end - 1)?;
    let mut date = first;
    let mut starts = Vec::new();
    loop {
        if let Some(instant) = local_day_start(date, timezone) {
            starts.push((date.to_string(), instant));
            if date > last {
                break;
            }
        }
        date = date
            .checked_add_days(Days::new(1))
            .ok_or_else(|| Error::Config("usage date is out of range".into()))?;
    }
    Ok(starts
        .windows(2)
        .map(|pair| (pair[0].0.clone(), pair[0].1, pair[1].1))
        .collect())
}

fn local_day_start(date: NaiveDate, timezone: Tz) -> Option<i64> {
    // Some zones change at midnight: pick the first existent minute on that civil day.
    (0..1440).find_map(|minute| {
        timezone
            .from_local_datetime(&date.and_hms_opt(minute / 60, minute % 60, 0)?)
            .earliest()
            .map(|value| value.timestamp_millis())
    })
}

impl Store {
    pub async fn usage_calendar(
        &self,
        start: i64,
        end: i64,
        models: Option<&str>,
        timezone: &str,
    ) -> Result<Vec<UsageCalendarDay>> {
        validate_range(Some(start), Some(end), models, None)?;
        let timezone: Tz = timezone
            .parse()
            .map_err(|_| Error::Config("unknown IANA timezone".into()))?;
        let boundaries = calendar_boundaries(start, end, timezone)?;
        let rows = sqlx::query(&format!(
            "SELECT json_extract(b.value, '$[0]') AS date,
                json_extract(b.value, '$[1]') AS start_ms,
                json_extract(b.value, '$[2]') AS end_ms,
                COUNT(CASE WHEN c.status != 'running' THEN c.call_id END) AS calls,
                COALESCE(SUM({fresh}), 0) AS input_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_read_tokens, 0))), 0) AS cache_read_tokens,
                COALESCE(SUM(MAX(0, COALESCE(cache_write_tokens, 0))), 0) AS cache_write_tokens,
                COALESCE(SUM(MAX(0, COALESCE(output_tokens, 0))), 0) AS output_tokens
             FROM json_each(?) b LEFT JOIN llm_calls c
               ON c.created_at_ms >= MAX(json_extract(b.value, '$[1]'), ?)
              AND c.created_at_ms < MIN(json_extract(b.value, '$[2]'), ?)
              AND (? IS NULL OR c.model_hash IN (SELECT value FROM json_each(?)))
             GROUP BY b.key ORDER BY start_ms",
            fresh = fresh_input_sql()
        ))
        .bind(serde_json::to_string(&boundaries)?)
        .bind(start)
        .bind(end)
        .bind(models)
        .bind(models)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(UsageCalendarDay {
                    date: row.try_get("date")?,
                    end_ms: row.try_get("end_ms")?,
                    calls: row.try_get("calls")?,
                    usage: TokenUsageBucket {
                        bucket_start_ms: row.try_get("start_ms")?,
                        input_tokens: row.try_get("input_tokens")?,
                        cache_read_tokens: row.try_get("cache_read_tokens")?,
                        cache_write_tokens: row.try_get("cache_write_tokens")?,
                        output_tokens: row.try_get("output_tokens")?,
                    },
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(value: &str) -> i64 {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }
    #[test]
    fn explicit_range_keeps_more_than_sixty_days() {
        let (_, _, _, count) = token_usage_buckets(Some(0), Some(365 * DAY_MS), None).unwrap();
        assert_eq!(count, 365);
        assert!(token_usage_buckets(Some(0), Some(365 * DAY_MS), Some(MINUTE_MS)).is_err());
    }
    #[test]
    fn calendar_respects_dst_and_non_whole_hour_offsets() {
        for (start, end, hours) in [
            ("2026-03-08T08:00:00Z", "2026-03-09T07:00:00Z", 23),
            ("2026-11-01T07:00:00Z", "2026-11-02T08:00:00Z", 25),
        ] {
            let buckets =
                calendar_boundaries(ms(start), ms(end), chrono_tz::America::Los_Angeles).unwrap();
            assert_eq!(buckets.len(), 1);
            assert_eq!(buckets[0].2 - buckets[0].1, hours * HOUR_MS);
        }
        let buckets = calendar_boundaries(
            ms("2026-01-01T18:15:00Z"),
            ms("2026-01-02T18:15:00Z"),
            chrono_tz::Asia::Kathmandu,
        )
        .unwrap();
        assert_eq!(buckets[0].0, "2026-01-02");
        assert_eq!(buckets.len(), 1);
    }
    #[test]
    fn rejects_invalid_filters_and_partial_ranges() {
        assert!(validate_range(Some(0), None, None, None).is_err());
        assert!(validate_range(Some(1), Some(1), None, None).is_err());
        assert!(validate_range(None, None, Some("{}"), None).is_err());
    }
}
