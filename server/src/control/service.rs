//! Focused Cursor/Sub2API control services.
use crate::{
    local_app::CursorHarness,
    model::{
        ContentPart, CursorRunTraceArtifact, CursorRunTraceSummary, LlmCallRequest,
        LlmCallResponseChunk, LlmCallSummary, ModelConfig, ModelInvocation, ModelRequest,
        ModelSpec, Overview, ProjectedContent, ProjectedMessage, PromptSpec, Role,
    },
    provider::{is_valid_response_event, ModelEvent, Provider},
    store::Store,
    Error, Result,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct ControlService {
    pub(super) store: Store,
    cursor_harness: CursorHarness,
    provider: Arc<dyn Provider>,
    model_tests: Arc<Mutex<ModelTestRegistry>>,
}

// A test ID is scoped to its model. Short-lived tombstones make a DELETE that
// races ahead of POST effective, and prevent replay of a completed POST.
#[derive(Default)]
struct ModelTestRegistry {
    entries: BTreeMap<(String, String), ModelTestEntry>,
}
struct ModelTestEntry {
    token: CancellationToken,
    started: bool,
    expires: Option<Instant>,
}
impl ModelTestRegistry {
    const RETAIN: std::time::Duration = std::time::Duration::from_secs(60);
    fn key(model: &str, id: &str) -> Result<(String, String)> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(Error::Config(
                "model test ID must contain 1–128 ASCII letters, digits, hyphens or underscores"
                    .into(),
            ));
        }
        Ok((model.to_owned(), id.to_owned()))
    }
    fn prune(&mut self) {
        let now = Instant::now();
        self.entries
            .retain(|_, entry| entry.expires.is_none_or(|expiry| expiry > now));
    }
    fn reserve(&mut self, key: &(String, String)) -> Result<&mut ModelTestEntry> {
        self.prune();
        if !self.entries.contains_key(key) && self.entries.len() >= 1024 {
            return Err(Error::Config(
                "too many model tests; please retry later".into(),
            ));
        }
        Ok(self
            .entries
            .entry(key.clone())
            .or_insert_with(|| ModelTestEntry {
                token: CancellationToken::new(),
                started: false,
                expires: Some(Instant::now() + Self::RETAIN),
            }))
    }
    fn start(&mut self, key: &(String, String)) -> Result<CancellationToken> {
        let entry = self.reserve(key)?;
        if entry.started {
            return Err(Error::ControllerConflict {
                code: "model_test_already_started",
                message: "model test ID already used; create a new test ID".into(),
            });
        }
        if entry.token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        entry.started = true;
        entry.expires = None;
        Ok(entry.token.clone())
    }
    fn cancel(&mut self, key: &(String, String)) -> Result<()> {
        self.reserve(key)?.token.cancel();
        Ok(())
    }
}
struct ModelTestLease {
    registry: Arc<Mutex<ModelTestRegistry>>,
    key: (String, String),
}
impl Drop for ModelTestLease {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            if let Some(entry) = registry.entries.get_mut(&self.key) {
                entry.token.cancel();
                entry.expires = Some(Instant::now() + ModelTestRegistry::RETAIN);
            }
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ModelConnectivityResult {
    pub duration_ms: u64,
    pub first_valid_response_ms: Option<u64>,
    pub output_tokens: u64,
    pub tokens_per_second: f64,
    pub tokens_estimated: bool,
    pub output: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallDetail {
    pub call: CallSummary,
    pub request: Option<LlmCallRequest>,
    pub response_chunks: Vec<LlmCallResponseChunk>,
    pub cursor_trace: Option<CursorTraceDetail>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CallSummary {
    #[serde(flatten)]
    pub call: LlmCallSummary,
    pub call_kind: &'static str,
    pub route: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct CursorTraceDetail {
    pub trace: CursorRunTraceSummary,
    pub artifacts: Vec<CursorTraceArtifactDetail>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CursorTraceArtifactDetail {
    pub seq: i64,
    pub artifact_type: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub created_at_ms: i64,
    pub byte_count: usize,
    pub encoding: &'static str,
    pub data: String,
}

impl ControlService {
    pub fn new(
        store: Store,
        provider: Arc<dyn Provider>,
        paths: &crate::config::RuntimePaths,
    ) -> Result<Self> {
        Ok(Self {
            cursor_harness: CursorHarness::new(
                store.clone(),
                paths.cursor_settings.clone(),
                paths.data_dir.clone(),
            )?,
            store,
            provider,
            model_tests: Arc::new(Mutex::new(ModelTestRegistry::default())),
        })
    }
    pub fn cursor_harness(&self) -> &CursorHarness {
        &self.cursor_harness
    }
    pub async fn connection(&self) -> Result<crate::store::Sub2ApiConnection> {
        self.store.sub2api_connection().await
    }
    pub async fn save_connection(
        &self,
        input: crate::store::Sub2ApiConnectionInput,
    ) -> Result<crate::store::Sub2ApiConnection> {
        let _guard = self.cursor_harness.configuration_guard().await?;
        self.store.set_sub2api_connection(input).await
    }
    pub async fn models(&self) -> Result<Vec<ModelConfig>> {
        self.store.models().await
    }

    pub async fn overview(
        &self,
        start_ms: Option<i64>,
        end_ms: Option<i64>,
        model_hashes: Option<&str>,
        bucket_ms: Option<i64>,
    ) -> Result<Overview> {
        self.store
            .overview(start_ms, end_ms, model_hashes, bucket_ms)
            .await
    }

    pub async fn create_models(
        &self,
        models: &[crate::store::Sub2ApiModelInput],
    ) -> Result<Vec<ModelConfig>> {
        let _guard = self.cursor_harness.configuration_guard().await?;
        let mut configured = Vec::new();
        for model in models {
            configured.push(self.store.sub2api_model_input(model).await?);
        }
        self.store.create_models(&configured).await
    }

    pub async fn reorder_models(&self, model_hashes: &[String]) -> Result<Vec<ModelConfig>> {
        let _guard = self.cursor_harness.configuration_guard().await?;
        self.store.reorder_models(model_hashes).await
    }

    pub async fn delete_model(&self, model_hash: &str) -> Result<()> {
        let _guard = self.cursor_harness.configuration_guard().await?;
        self.store.delete_model(model_hash).await
    }

    pub async fn update_model(
        &self,
        model_hash: &str,
        input: &crate::store::Sub2ApiModelInput,
    ) -> Result<ModelConfig> {
        let _guard = self.cursor_harness.configuration_guard().await?;
        self.store
            .update_model(model_hash, &self.store.sub2api_model_input(input).await?)
            .await
    }

    pub async fn test_model(
        &self,
        model_hash: &str,
        test_id: &str,
    ) -> Result<ModelConnectivityResult> {
        let key = ModelTestRegistry::key(model_hash, test_id)?;
        let cancellation = self
            .model_tests
            .lock()
            .expect("model test registry mutex poisoned")
            .start(&key)?;
        let _lease = ModelTestLease {
            registry: self.model_tests.clone(),
            key,
        };
        self.run_model_test(model_hash, cancellation).await
    }

    pub fn cancel_model_test(&self, model_hash: &str, test_id: &str) -> Result<()> {
        let key = ModelTestRegistry::key(model_hash, test_id)?;
        self.model_tests
            .lock()
            .expect("model test registry mutex poisoned")
            .cancel(&key)
    }

    async fn run_model_test(
        &self,
        model_hash: &str,
        cancellation: CancellationToken,
    ) -> Result<ModelConnectivityResult> {
        const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
        const TEST_PROMPT: &str = "Output the numbers 1 through 120 separated by a single space. No commas, no newlines, no explanation.";

        let mut model = ModelSpec::new(model_hash);
        let configured = self
            .store
            .model(model_hash)
            .await?
            .ok_or_else(|| Error::RunNotFound("model".into()))?;
        configured.configure(&mut model);
        model.max_output_tokens = Some(configured.max_output_tokens().unwrap_or(256).min(256));
        let call_id = format!("model-test-{}", uuid::Uuid::new_v4());
        let invocation = ModelInvocation {
            call_id: call_id.clone(),
            run_id: call_id.clone(),
            conversation_id: call_id.clone(),
            provider_call_index: 0,
            request: ModelRequest {
                prompt: PromptSpec {
                    instructions: String::new(),
                    tools: Vec::new(),
                },
                model,
                history: vec![ProjectedMessage {
                    message_id: "connectivity-test".into(),
                    role: Role::User,
                    content: ProjectedContent::Parts(vec![ContentPart::Text {
                        text: TEST_PROMPT.into(),
                    }]),
                }],
            },
        };
        let started = Instant::now();
        let mut first_valid_response_at = None;
        let mut output_tokens = None;
        let mut output = String::new();
        let stream = self.provider.stream(invocation, cancellation.clone());
        let completed = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Ok(Err(Error::Cancelled)),
            result = tokio::time::timeout(TEST_TIMEOUT, async {
            futures_util::pin_mut!(stream);
            let mut finished = false;
            while let Some(event) = stream.next().await {
                let event = event?;
                if first_valid_response_at.is_none() && is_valid_response_event(&event) {
                    first_valid_response_at = Some(Instant::now());
                }
                match event {
                    ModelEvent::TextDelta(delta) => {
                        output.push_str(&delta);
                    }
                    ModelEvent::Usage(usage) => {
                        if let Some(tokens) = usage.output_tokens.filter(|tokens| *tokens > 0) {
                            output_tokens = Some(
                                output_tokens.map_or(tokens, |current: u64| current.max(tokens)),
                            );
                        }
                    }
                    ModelEvent::Done(_) => finished = true,
                    _ => {}
                }
            }
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            if !finished {
                return Err(Error::Protocol(
                    "provider stream ended without Done during connectivity test".into(),
                ));
            }
            Ok(())
            }) => result,
        };
        match completed {
            Ok(Err(Error::Cancelled)) => {
                self.store
                    .finish_llm_call(
                        &call_id,
                        "cancelled",
                        None,
                        started.elapsed().as_millis().min(i64::MAX as u128) as i64,
                        Some("cancelled"),
                        Some("model connectivity test cancelled"),
                    )
                    .await?;
                return Err(Error::Cancelled);
            }
            Ok(result) => result?,
            Err(_) => {
                cancellation.cancel();
                self.store
                    .finish_llm_call(
                        &call_id,
                        "error",
                        None,
                        started.elapsed().as_millis().min(i64::MAX as u128) as i64,
                        Some("timeout"),
                        Some("model connectivity test timed out after 45 seconds"),
                    )
                    .await?;
                return Err(Error::Provider(
                    "model connectivity test timed out after 45 seconds".into(),
                ));
            }
        }
        let elapsed = started.elapsed();
        let output = output.trim().to_string();
        if first_valid_response_at.is_none() {
            return Err(Error::Provider(
                "model connectivity test received no valid response".into(),
            ));
        }
        let tokens_estimated = output_tokens.is_none();
        let output_tokens = output_tokens.unwrap_or_else(|| estimate_output_tokens(&output));
        Ok(ModelConnectivityResult {
            duration_ms: elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
            first_valid_response_ms: first_valid_response_at.map(|first| {
                first
                    .duration_since(started)
                    .as_millis()
                    .min(u128::from(u64::MAX)) as u64
            }),
            output_tokens,
            tokens_per_second: if elapsed.is_zero() {
                0.0
            } else {
                output_tokens as f64 / elapsed.as_secs_f64()
            },
            tokens_estimated,
            output,
        })
    }

    pub async fn calls(&self, limit: i64) -> Result<Vec<CallSummary>> {
        let mut calls = self
            .store
            .llm_calls(limit)
            .await?
            .into_iter()
            .map(|call| {
                let route = call_route(&call.run_id);
                CallSummary {
                    call,
                    call_kind: "provider_llm",
                    route,
                }
            })
            .collect::<Vec<_>>();
        calls.extend(
            self.store
                .official_cursor_traces(limit)
                .await?
                .into_iter()
                .map(official_call),
        );
        calls.sort_by_key(|call| std::cmp::Reverse(call.call.created_at_ms));
        calls.truncate(limit.clamp(1, 500) as usize);
        Ok(calls)
    }

    pub async fn call(&self, call_id: &str) -> Result<CallDetail> {
        if let Some(call) = self.store.llm_call(call_id).await? {
            let cursor_trace = self.cursor_trace_detail(&call.run_id).await?;
            let route = call_route(&call.run_id);
            return Ok(CallDetail {
                request: self.store.llm_call_request(call_id).await?,
                response_chunks: self.store.llm_call_chunks(call_id).await?,
                call: CallSummary {
                    call,
                    call_kind: "provider_llm",
                    route,
                },
                cursor_trace,
            });
        }
        let request_id = call_id.strip_prefix("cursor:").unwrap_or(call_id);
        let trace = self
            .store
            .cursor_trace(request_id)
            .await?
            .filter(|trace| trace.route == "cursor_official")
            .ok_or_else(|| Error::RunNotFound(format!("call {call_id}")))?;
        Ok(CallDetail {
            call: official_call(trace.clone()),
            request: None,
            response_chunks: Vec::new(),
            cursor_trace: Some(self.cursor_trace_detail_from(trace).await?),
        })
    }

    async fn cursor_trace_detail(&self, request_id: &str) -> Result<Option<CursorTraceDetail>> {
        let Some(trace) = self.store.cursor_trace(request_id).await? else {
            return Ok(None);
        };
        Ok(Some(self.cursor_trace_detail_from(trace).await?))
    }

    async fn cursor_trace_detail_from(
        &self,
        trace: CursorRunTraceSummary,
    ) -> Result<CursorTraceDetail> {
        let artifacts = self
            .store
            .cursor_trace_artifacts(&trace.request_id)
            .await?
            .into_iter()
            .map(cursor_artifact)
            .collect();
        Ok(CursorTraceDetail { trace, artifacts })
    }
}
fn call_route(run_id: &str) -> &'static str {
    if run_id.starts_with("external-api:") {
        "external_api"
    } else {
        "local_byok"
    }
}

fn official_call(trace: CursorRunTraceSummary) -> CallSummary {
    let model_id = trace.model_id.clone().unwrap_or_else(|| "Cursor".into());
    let ttfb = trace
        .first_response_at_ms
        .map(|value| (value - trace.received_at_ms).max(0));
    let duration = trace
        .finished_at_ms
        .map(|value| (value - trace.received_at_ms).max(0));
    let error = trace.error_message.clone();
    CallSummary {
        call: LlmCallSummary {
            call_id: format!("cursor:{}", trace.request_id),
            run_id: trace.request_id.clone(),
            conversation_id: trace
                .conversation_id
                .clone()
                .unwrap_or_else(|| trace.request_id.clone()),
            provider_call_index: 0,
            model_hash: None,
            provider_type: "cursor-official".into(),
            provider_url: "https://api2.cursor.sh".into(),
            request_type: "cursor-run-sse".into(),
            request_url: "https://api2.cursor.sh/agent.v1.AgentService/RunSSE".into(),
            model_id: model_id.clone(),
            display_name: model_id,
            reasoning_effort: None,
            fast: None,
            status: trace.status.clone(),
            finish_reason: None,
            created_at_ms: trace.received_at_ms,
            request_started_at_ms: Some(trace.received_at_ms),
            response_headers_at_ms: trace.first_response_at_ms,
            first_event_at_ms: trace.first_response_at_ms,
            first_text_at_ms: None,
            first_valid_response_at_ms: None,
            finished_at_ms: trace.finished_at_ms,
            queue_ms: None,
            ttfb_ms: ttfb,
            ttft_ms: None,
            ttfr_ms: None,
            duration_ms: duration,
            input_tokens: None,
            output_tokens: None,
            total_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
            usage: None,
            message_count: 0,
            tool_count: 0,
            request_bytes: Some(trace.request_bytes),
            response_bytes: trace.response_bytes,
            stream_event_count: trace.response_event_count,
            http_status: trace.http_status,
            error_kind: error.as_ref().map(|_| "cursor_official".into()),
            error_message: error,
            detailed: true,
        },
        call_kind: "cursor_official",
        route: "cursor_official",
    }
}

fn cursor_artifact(artifact: CursorRunTraceArtifact) -> CursorTraceArtifactDetail {
    let byte_count = artifact.data.len();
    let (encoding, data) = match readable_utf8(&artifact.data) {
        Some(value) => ("utf8", value.into()),
        None => ("base64", STANDARD.encode(&artifact.data)),
    };
    CursorTraceArtifactDetail {
        seq: artifact.seq,
        artifact_type: artifact.artifact_type,
        source: artifact.source,
        metadata: artifact.metadata,
        created_at_ms: artifact.created_at_ms,
        byte_count,
        encoding,
        data,
    }
}

fn readable_utf8(data: &[u8]) -> Option<&str> {
    let value = std::str::from_utf8(data).ok()?;
    value
        .chars()
        .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .then_some(value)
}

fn estimate_output_tokens(output: &str) -> u64 {
    let words = output.split_whitespace().count() as u64;
    if words > 0 {
        words
    } else if output.is_empty() {
        0
    } else {
        (output.chars().count() as u64).div_ceil(4)
    }
}

#[cfg(test)]
mod model_management_tests {
    use super::*;

    #[test]
    fn cancellation_is_scoped_and_precedes_dispatch() {
        let id = uuid::Uuid::new_v4().to_string();
        let a = ModelTestRegistry::key("a", &id).unwrap();
        let b = ModelTestRegistry::key("b", &id).unwrap();
        let mut tests = ModelTestRegistry::default();
        let running_a = tests.start(&a).unwrap();
        tests.cancel(&b).unwrap();
        assert!(!running_a.is_cancelled());
        assert!(matches!(tests.start(&b), Err(Error::Cancelled)));
        assert!(matches!(
            tests.start(&a),
            Err(Error::ControllerConflict { .. })
        ));
        tests.cancel(&a).unwrap();
        assert!(running_a.is_cancelled());
    }

    #[test]
    fn dropped_test_cancels_and_replay_is_rejected_until_tombstone_expires() {
        let registry = Arc::new(Mutex::new(ModelTestRegistry::default()));
        let key = ModelTestRegistry::key("a", &uuid::Uuid::new_v4().to_string()).unwrap();
        let token = registry.lock().unwrap().start(&key).unwrap();
        drop(ModelTestLease {
            registry: registry.clone(),
            key: key.clone(),
        });
        assert!(token.is_cancelled());
        let mut registry = registry.lock().unwrap();
        assert!(matches!(
            registry.start(&key),
            Err(Error::ControllerConflict { .. })
        ));
        registry.entries.get_mut(&key).unwrap().expires = Some(Instant::now());
        registry.prune();
        assert!(registry.entries.is_empty());
        assert!(ModelTestRegistry::key("a", "").is_err());
        assert!(ModelTestRegistry::key("a", &"x".repeat(129)).is_err());
    }
}
