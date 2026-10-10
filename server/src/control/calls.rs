//! Paged metadata inspection. Never loads conversation bodies or credentials.
use super::{CallDetail, ControlService};
use crate::{model::LlmCallSummary, store::RouteDiagnostic, Error, Result};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row, Sqlite};

#[derive(Default, Deserialize)]
pub struct CallQuery {
    page: Option<i64>,
    limit: Option<i64>,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
    model_hash: Option<String>,
    status: Option<String>,
    conversation_id: Option<String>,
}
#[derive(Serialize)]
pub struct CallRow {
    #[serde(flatten)]
    call: LlmCallSummary,
    request_id: Option<String>,
    parent_request_id: Option<String>,
    parent_tool_call_id: Option<String>,
    method: &'static str,
    route: &'static str,
}
#[derive(Serialize)]
pub struct CallPage {
    items: Vec<CallRow>,
    total: i64,
    page: i64,
    limit: i64,
}

fn filters<'a>(q: &'a CallQuery, sql: &mut QueryBuilder<'a, Sqlite>) {
    sql.push(" WHERE 1=1");
    if let Some(v) = q.start_ms {
        sql.push(" AND c.created_at_ms >= ").push_bind(v);
    }
    if let Some(v) = q.end_ms {
        sql.push(" AND c.created_at_ms < ").push_bind(v);
    }
    if let Some(v) = &q.model_hash {
        sql.push(" AND c.model_hash = ").push_bind(v);
    }
    if let Some(v) = &q.status {
        match v.as_str() {
            "success" => {
                sql.push(" AND c.status = 'completed'");
            }
            "failed" => {
                sql.push(" AND c.status IN ('failed','error','cancelled')");
            }
            "running" => {
                sql.push(" AND c.status = 'running'");
            }
            _ => {}
        }
    }
    if let Some(v) = &q.conversation_id {
        sql.push(" AND c.conversation_id = ").push_bind(v);
    }
}
pub async fn list(
    State(service): State<ControlService>,
    Query(q): Query<CallQuery>,
) -> Result<Json<CallPage>> {
    if q.start_ms.zip(q.end_ms).is_some_and(|(s, e)| s >= e)
        || q.status
            .as_deref()
            .is_some_and(|s| !matches!(s, "success" | "failed" | "running"))
    {
        return Err(Error::Config("invalid call filter".into()));
    }
    let page = q.page.unwrap_or(1).clamp(1, 1_000_000);
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let mut tx = service.store.pool().begin().await?;
    let mut count = QueryBuilder::new("SELECT COUNT(*) FROM llm_calls c");
    filters(&q, &mut count);
    let total: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
    let mut sql=QueryBuilder::new("SELECT c.*,r.cursor_request_id AS request_id, (SELECT parent_request_id FROM route_diagnostics d WHERE d.request_id=r.cursor_request_id AND d.parent_request_id IS NOT NULL ORDER BY d.id DESC LIMIT 1) AS parent_request_id, (SELECT parent_tool_call_id FROM route_diagnostics d WHERE d.request_id=r.cursor_request_id AND d.parent_tool_call_id IS NOT NULL ORDER BY d.id DESC LIMIT 1) AS parent_tool_call_id FROM llm_calls c LEFT JOIN runs r ON r.run_id=c.run_id");
    filters(&q, &mut sql);
    sql.push(" ORDER BY c.created_at_ms DESC,c.call_id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind((page - 1) * limit);
    let rows = sql.build().fetch_all(&mut *tx).await?;
    tx.commit().await?;
    let items = rows
        .into_iter()
        .map(|r| {
            let request_id = r.try_get("request_id")?;
            let parent_request_id = r.try_get("parent_request_id")?;
            let parent_tool_call_id = r.try_get("parent_tool_call_id")?;
            let mut call = crate::store::call_summary_from_row(r)?;
            call.error_message = call.error_message.as_deref().map(safe_error);
            call.error_kind = call.error_kind.filter(|s| {
                s.len() < 80 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            });
            call.provider_url = safe_url(&call.provider_url);
            call.request_url = safe_url(&call.request_url);
            Ok(CallRow {
                call,
                request_id,
                parent_request_id,
                parent_tool_call_id,
                method: "POST",
                route: "sub2api",
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Json(CallPage {
        items,
        total,
        page,
        limit,
    }))
}
fn safe_url(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .map(|mut u| {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        })
        .unwrap_or_default()
}
fn safe_error(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    if lower.contains("insufficient") && (lower.contains("credit") || lower.contains("balance")) {
        "上游余额或额度不足"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "连接或响应超时"
    } else if lower.contains("cancel") {
        "请求已取消"
    } else if lower.contains("429") || lower.contains("rate limit") {
        "上游限流"
    } else if lower.contains("401") || lower.contains("unauthorized") {
        "上游身份验证失败"
    } else {
        "请求失败；原始错误可能包含敏感内容，未在此展示"
    }
    .into()
}

#[derive(Deserialize)]
pub struct DiagnosticQuery {
    request_id: Option<String>,
    before: Option<i64>,
}
pub async fn diagnostics(
    State(s): State<ControlService>,
    Query(q): Query<DiagnosticQuery>,
) -> Result<Json<Vec<RouteDiagnostic>>> {
    Ok(Json(
        s.store
            .route_diagnostics(q.request_id.as_deref(), q.before)
            .await?,
    ))
}
pub async fn detail(
    State(service): State<ControlService>,
    Path(id): Path<String>,
) -> Result<Json<CallDetail>> {
    Ok(Json(service.call(&id).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_redaction_never_echoes_error_payload_or_url_secrets() {
        assert!(!safe_error("401 Bearer private-key: private conversation").contains("private"));
        assert_eq!(
            safe_url("https://user:secret@example.com/v1/responses?token=private#private"),
            "https://example.com/v1/responses"
        );
    }
}
