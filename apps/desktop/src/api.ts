declare global { interface Window { readonly __SUB2API_CONTROL_TOKEN__?: string } }
export type Model = { sort_order: number; group_name: string | null; model_hash: string; display_name: string; model_id: string; type: "openai" | "anthropic"; openai_endpoint: string; reasoning_effort: string | null; allowed_reasoning_efforts: string[]; anthropic_thinking_effort: string | null; context_window_tokens: number | null; max_completion_tokens: number | null; thinking_budget_tokens: number | null };
export type Status = { integration: "disabled" | "enabled" | "degraded" | "recovery_required"; ca: "missing" | "untrusted" | "ready" | "invalid"; ca_sha256: string | null; certificate_consent: boolean; settings_path: string; proxy_url: string | null; restart_required: boolean; recovery_error: string | null; warnings: string[]; subscription_injected: boolean; subscription_recovery_pending: boolean };
export type Connection = { base_url: string; has_api_key: boolean };
export async function request<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const token = window.__SUB2API_CONTROL_TOKEN__;
  if (!token) throw new Error("请从 Cursor Sub2API BYOK 桌面程序打开控制面板。");
  const response = await fetch(`/__byok-api__/api/${path}`, {
    method, headers: { "Content-Type": "application/json", "X-Sub2API-Control-Token": token },
    body: body === undefined ? undefined : JSON.stringify(body), credentials: "omit", cache: "no-store",
  });
  if (!response.ok) {
    const error = await response.json().catch(() => null) as { message?: string } | null;
    throw new Error(error?.message || `请求失败 (${response.status})`);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}

export const efforts = ["low", "medium", "high", "xhigh", "max"];
export const defaultEffort = (model: Model) => (model.type === "anthropic" ? model.anthropic_thinking_effort : model.reasoning_effort);
export const protocol = (model: Model) => model.type === "anthropic" ? "Messages" : model.openai_endpoint.includes("responses") ? "Responses" : "Chat Completions";
export type ModelInput = { sort_order?: number; group_name?: string | null; display_name: string; model_id: string; type: Model["type"]; openai_endpoint?: string; reasoning_effort: string | null; allowed_reasoning_efforts: string[]; context_window_tokens?: number | null; max_completion_tokens?: number | null; thinking_budget_tokens?: number | null };
export type Metrics = { llm_calls: number; successful_calls: number; failed_calls: number; token_usage: number; input_tokens: number; cache_read_tokens: number; cache_write_tokens: number; output_tokens: number };
export type UsageBucket = { bucket_start_ms: number; input_tokens: number; cache_read_tokens: number; cache_write_tokens: number; output_tokens: number };
export type Overview = { metrics: Metrics; token_usage_granularity: "day" | "hour" | "minute"; token_usage_series: UsageBucket[] };
