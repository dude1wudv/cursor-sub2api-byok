//! Metadata only: no bodies, headers, credentials or arbitrary error text.
use super::{now_ms, Store};
use crate::Result;
use serde::Serialize;
use sqlx::Row;

#[derive(Default, Serialize)]
pub struct RouteDiagnostic {
    pub id: i64,
    pub created_at_ms: i64,
    pub request_id: Option<String>,
    pub conversation_id: Option<String>,
    pub parent_request_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub method: String,
    pub path: String,
    pub route: String,
    pub stage: String,
    pub http_status: u16,
    pub duration_ms: i64,
}

impl Store {
    pub async fn record_route_diagnostic(&self, d: &RouteDiagnostic) -> Result<()> {
        let _write = self.writes.lock().await;
        sqlx::query("INSERT INTO route_diagnostics(created_at_ms,request_id,conversation_id,parent_request_id,parent_tool_call_id,method,path,route,stage,http_status,duration_ms) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(now_ms()).bind(identifier(d.request_id.as_deref())).bind(identifier(d.conversation_id.as_deref()))
            .bind(identifier(d.parent_request_id.as_deref())).bind(identifier(d.parent_tool_call_id.as_deref()))
            .bind(&d.method).bind(&d.path).bind(&d.route).bind(&d.stage).bind(i64::from(d.http_status)).bind(d.duration_ms)
            .execute(&self.pool).await?;
        sqlx::query("DELETE FROM route_diagnostics WHERE id <= (SELECT MAX(id)-10000 FROM route_diagnostics)").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn route_diagnostics(
        &self,
        request_id: Option<&str>,
        before: Option<i64>,
    ) -> Result<Vec<RouteDiagnostic>> {
        let rows = sqlx::query("SELECT * FROM route_diagnostics WHERE (? IS NULL OR request_id=? OR parent_request_id=?) AND (? IS NULL OR id < ?) ORDER BY id DESC LIMIT 100")
            .bind(request_id).bind(request_id).bind(request_id).bind(before).bind(before).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|r| {
                Ok(RouteDiagnostic {
                    id: r.try_get("id")?,
                    created_at_ms: r.try_get("created_at_ms")?,
                    request_id: r.try_get("request_id")?,
                    conversation_id: r.try_get("conversation_id")?,
                    parent_request_id: r.try_get("parent_request_id")?,
                    parent_tool_call_id: r.try_get("parent_tool_call_id")?,
                    method: r.try_get("method")?,
                    path: r.try_get("path")?,
                    route: r.try_get("route")?,
                    stage: r.try_get("stage")?,
                    http_status: r.try_get::<i64, _>("http_status")? as u16,
                    duration_ms: r.try_get("duration_ms")?,
                })
            })
            .collect()
    }
}

fn identifier(value: Option<&str>) -> Option<&str> {
    value.filter(|v| {
        !v.is_empty()
            && v.len() <= 160
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
    })
}
