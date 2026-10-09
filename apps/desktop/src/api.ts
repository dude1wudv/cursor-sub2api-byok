declare global { interface Window { readonly __SUB2API_CONTROL_TOKEN__?: string } }
export type Model = { model_hash: string; display_name: string; model_id: string; type: "openai" | "anthropic"; openai_endpoint: string; reasoning_effort: string | null; anthropic_thinking_effort: string | null; context_window_tokens: number | null; max_completion_tokens: number | null; thinking_budget_tokens: number | null };
export type Status = { integration: "disabled" | "enabled" | "degraded" | "recovery_required"; ca: string; ca_sha256: string | null; settings_path: string; proxy_url: string | null; restart_required: boolean; recovery_error: string | null; warnings: string[] };
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
