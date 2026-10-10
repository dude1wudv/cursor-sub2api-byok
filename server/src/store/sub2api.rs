//! One DPAPI connection and atomic propagation to every configured model.
use super::{now_ms, Store};
use crate::{
    local_app::secrets,
    model::{
        model_hash, normalize_model_input, ModelConfig, ModelConfigInput, ModelType,
        OPENAI_CHAT_ENDPOINT, OPENAI_RESPONSES_ENDPOINT,
    },
    Error, Result,
};
use serde::{Deserialize, Serialize};
const KEY: &str = "sub2api_connection";
pub(super) const SHARED_KEY_REFERENCE: &str = "sub2api-connection:v1";
#[derive(Clone, Debug, Serialize)]
pub struct Sub2ApiConnection {
    pub base_url: String,
    pub has_api_key: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sub2ApiConnectionInput {
    pub base_url: String,
    pub api_key: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct SavedConnection {
    base_url: String,
    api_key_protected: String,
}

// Model rows reference the one encrypted connection. Reject stale inputs rather
// than accidentally persisting another encrypted copy after a connection change.
pub(super) async fn stored_model_key(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    input: &ModelConfigInput,
) -> Result<String> {
    let saved: Option<String> =
        sqlx::query_scalar("SELECT value_json FROM service_settings WHERE setting_key = ?")
            .bind(KEY)
            .fetch_optional(&mut **transaction)
            .await?;
    let Some(saved) = saved else {
        return secrets::protect_string(&input.api_key);
    };
    let saved: SavedConnection = serde_json::from_str(&saved)
        .map_err(|_| Error::Config("saved Sub2API connection is invalid".into()))?;
    if input.base_url != saved.base_url
        || input.api_key != secrets::unprotect_string(&saved.api_key_protected)?
    {
        return Err(Error::Config(
            "model input does not match the shared Sub2API connection; refresh and retry".into(),
        ));
    }
    Ok(SHARED_KEY_REFERENCE.into())
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sub2ApiModelInput {
    pub display_name: String,
    pub model_id: String,
    #[serde(rename = "type")]
    pub model_type: ModelType,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default = "crate::model::default_reasoning_efforts")]
    pub allowed_reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub openai_endpoint: Option<String>,
    #[serde(default)]
    pub context_window_tokens: Option<u64>,
    #[serde(default)]
    pub max_completion_tokens: Option<u64>,
    #[serde(default)]
    pub thinking_budget_tokens: Option<u64>,
}
impl Sub2ApiModelInput {
    fn from_model(m: &ModelConfig) -> Self {
        Self {
            allowed_reasoning_efforts: m.allowed_reasoning_efforts.clone(),
            display_name: m.display_name.clone(),
            model_id: m.model_id.clone(),
            model_type: m.model_type,
            sort_order: m.sort_order,
            group_name: m.group_name.clone(),
            reasoning_effort: if m.model_type == ModelType::Anthropic {
                m.anthropic_thinking_effort.clone()
            } else {
                m.reasoning_effort.clone()
            },
            openai_endpoint: Some(m.openai_endpoint.clone()),
            context_window_tokens: m.context_window_tokens,
            max_completion_tokens: m.max_output_tokens(),
            thinking_budget_tokens: m.thinking_budget_tokens,
        }
    }
    fn configured(&self, base_url: &str, api_key: &str) -> Result<ModelConfigInput> {
        let endpoint = self
            .openai_endpoint
            .as_deref()
            .unwrap_or(OPENAI_RESPONSES_ENDPOINT);
        if self.model_type == ModelType::OpenAi
            && !matches!(endpoint, OPENAI_RESPONSES_ENDPOINT | OPENAI_CHAT_ENDPOINT)
        {
            return Err(Error::Config(
                "OpenAI endpoint must be Responses or Chat Completions".into(),
            ));
        }
        normalize_model_input(&ModelConfigInput {
            allowed_reasoning_efforts: self.allowed_reasoning_efforts.clone(),
            display_name: self.display_name.clone(),
            tooltip_data: self.display_name.clone(),
            model_id: self.model_id.clone(),
            model_type: self.model_type,
            sort_order: self.sort_order,
            group_name: self.group_name.clone(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            use_full_url: false,
            reasoning_effort: if self.model_type == ModelType::OpenAi {
                self.reasoning_effort.clone()
            } else {
                None
            },
            anthropic_thinking_effort: if self.model_type == ModelType::Anthropic {
                self.reasoning_effort.clone()
            } else {
                None
            },
            openai_endpoint: endpoint.into(),
            context_window_tokens: self.context_window_tokens,
            max_completion_tokens: self.max_completion_tokens,
            anthropic_max_tokens: self.max_completion_tokens,
            thinking_budget_tokens: self.thinking_budget_tokens,
            openai_extra_params_enabled: false,
            openai_extra_params: serde_json::json!({}),
            custom_headers_enabled: false,
            custom_headers: serde_json::json!({}),
            anthropic_extra_params_enabled: false,
            anthropic_extra_params: serde_json::json!({}),
        })
    }
}

fn normalize_base(value: &str) -> Result<String> {
    let url =
        url::Url::parse(value.trim()).map_err(|_| Error::Config("Sub2API Base URL 无效".into()))?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host == "localhost",
        _ => false,
    };
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path().trim_end_matches('/'), "" | "/v1")
    {
        return Err(Error::Config("Sub2API Base URL 仅支持 HTTPS 的根地址或 /v1；不允许凭据、查询、fragment 或完整 endpoint。".into()));
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

impl Store {
    async fn saved_connection(&self) -> Result<Option<SavedConnection>> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM service_settings WHERE setting_key = ?")
                .bind(KEY)
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|v| {
                serde_json::from_str(&v)
                    .map_err(|_| Error::Config("saved Sub2API connection is invalid".into()))
            })
            .transpose()
    }
    pub async fn sub2api_connection(&self) -> Result<Sub2ApiConnection> {
        Ok(match self.saved_connection().await? {
            Some(saved) => Sub2ApiConnection {
                base_url: saved.base_url,
                has_api_key: !saved.api_key_protected.is_empty(),
            },
            None => Sub2ApiConnection {
                base_url: String::new(),
                has_api_key: false,
            },
        })
    }
    pub(crate) async fn sub2api_credentials(&self) -> Result<(String, String)> {
        let saved = self
            .saved_connection()
            .await?
            .ok_or_else(|| Error::Config("请先保存 Sub2API 连接".into()))?;
        Ok((
            saved.base_url,
            secrets::unprotect_string(&saved.api_key_protected)?,
        ))
    }
    pub async fn sub2api_model_input(&self, input: &Sub2ApiModelInput) -> Result<ModelConfigInput> {
        let saved = self
            .saved_connection()
            .await?
            .ok_or_else(|| Error::Config("请先保存 Sub2API 连接".into()))?;
        input.configured(
            &saved.base_url,
            &secrets::unprotect_string(&saved.api_key_protected)?,
        )
    }
    pub async fn set_sub2api_connection(
        &self,
        input: Sub2ApiConnectionInput,
    ) -> Result<Sub2ApiConnection> {
        let base_url = normalize_base(&input.base_url)?;
        let _write = self.writes.lock().await;
        let key = match input.api_key {
            Some(key) if !key.trim().is_empty() => key.trim().to_owned(),
            Some(_) => {
                return Err(Error::Config(
                    "Sub2API Key 不能为空；省略表示保留已有 Key。".into(),
                ))
            }
            None => secrets::unprotect_string(
                &self
                    .saved_connection()
                    .await?
                    .ok_or_else(|| Error::Config("首次保存需要 Sub2API Key".into()))?
                    .api_key_protected,
            )?,
        };
        let saved = SavedConnection {
            base_url: base_url.clone(),
            api_key_protected: secrets::protect_string(&key)?,
        };
        let models = self.models().await?;
        let mut tx = self.pool.begin().await?;
        // Read callers still see the old snapshot until this transaction commits.
        // New model rows must resolve the new connection within this transaction.
        sqlx::query("INSERT INTO service_settings(setting_key,value_json,updated_at_ms) VALUES (?,?,?) ON CONFLICT(setting_key) DO UPDATE SET value_json=excluded.value_json,updated_at_ms=excluded.updated_at_ms")
            .bind(KEY).bind(serde_json::to_string(&saved)?).bind(now_ms()).execute(&mut *tx).await?;
        for old in models {
            let next = Sub2ApiModelInput::from_model(&old).configured(&base_url, &key)?;
            let hash = model_hash(&next)?;
            if hash != old.model_hash {
                super::models::insert_model(&mut tx, &hash, &next, now_ms()).await?;
                sqlx::query("UPDATE llm_calls SET model_hash = ? WHERE model_hash = ?")
                    .bind(&hash)
                    .bind(&old.model_hash)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM model_configs WHERE model_hash = ?")
                    .bind(&old.model_hash)
                    .execute(&mut *tx)
                    .await?;
            } else {
                // Also remove historical per-model ciphertext on a no-op save.
                sqlx::query(
                    "UPDATE model_configs SET api_key = ?, updated_at_ms = ? WHERE model_hash = ?",
                )
                .bind(SHARED_KEY_REFERENCE)
                .bind(now_ms())
                .bind(&old.model_hash)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(Sub2ApiConnection {
            base_url,
            has_api_key: true,
        })
    }
}
