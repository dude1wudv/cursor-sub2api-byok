import { useState } from "react";
import { defaultEffort, efforts, type Model, type ModelInput } from "./api";

export function ModelEditor({ model, cancel, save }: { model: Model | "new"; cancel: () => void; save: (input: ModelInput) => Promise<void> }) {
  const old = model === "new" ? null : model;
  const [name, setName] = useState(old?.display_name ?? "");
  const [id, setId] = useState(old?.model_id ?? "");
  const [type, setType] = useState<Model["type"]>(old?.type ?? "openai");
  const [endpoint, setEndpoint] = useState(old?.openai_endpoint || "/v1/responses");
  const [allowed, setAllowed] = useState(old?.allowed_reasoning_efforts ?? efforts);
  const [effort, setEffort] = useState(old ? defaultEffort(old) ?? "" : "");
  const [context, setContext] = useState(old?.context_window_tokens?.toString() ?? "");
  const [maxTokens, setMaxTokens] = useState(old?.max_completion_tokens?.toString() ?? "");
  const [group, setGroup] = useState(old?.group_name ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const toggle = (value: string) => {
    setAllowed(current => current.includes(value) ? current.filter(v => v !== value) : [...current, value]);
    if (effort === value) setEffort("");
  };
  return <div className="overlay"><form className="dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title" onSubmit={async e => {
    e.preventDefault(); setBusy(true); setError("");
    try {
      await save({ display_name: name, model_id: id, type, sort_order: old?.sort_order ?? 0, group_name: group.trim() || null, openai_endpoint: endpoint, reasoning_effort: effort || null, allowed_reasoning_efforts: allowed, context_window_tokens: context ? Number(context) : null, max_completion_tokens: maxTokens ? Number(maxTokens) : null, thinking_budget_tokens: old?.thinking_budget_tokens ?? null });
    } catch (e) { setError(e instanceof Error ? e.message : "保存失败"); }
    finally { setBusy(false); }
  }}>
    <div className="section-heading"><div><span className="eyebrow">MODEL CONFIGURATION</span><h2 id="editor-title">{old ? "编辑模型" : "手动添加模型"}</h2></div><button type="button" className="icon-button" aria-label="关闭" disabled={busy} onClick={cancel}>×</button></div>
    {error && <div className="alert error" role="alert">{error}</div>}
    <fieldset disabled={busy}>
      <div className="fields"><label>显示名称<input autoFocus required value={name} onChange={e => setName(e.target.value)} /></label><label>模型 ID<input required value={id} onChange={e => setId(e.target.value)} placeholder="与 Sub2API 模型 ID 一致" /></label></div>
      <label>分组<input maxLength={80} value={group} onChange={e => setGroup(e.target.value)} placeholder="例如：日常开发；留空为未分组" /></label>
      <div className="fields"><label>协议<select value={type} onChange={e => setType(e.target.value as Model["type"])}><option value="openai">OpenAI / GPT</option><option value="anthropic">Anthropic / Claude</option></select></label><label>Endpoint<select disabled={type === "anthropic"} value={type === "anthropic" ? "/v1/messages" : endpoint} onChange={e => setEndpoint(e.target.value)}>{type === "anthropic" ? <option value="/v1/messages">Messages</option> : <><option value="/v1/responses">Responses</option><option value="/v1/chat/completions">Chat Completions</option></>}</select></label></div>
      <div className="effort-section"><div className="section-heading"><h3>推理强度</h3><span>在 Cursor 中切换</span></div><span className="field-caption">可选强度</span><div className="effort-options">{efforts.map(value => <label className={`check-chip ${allowed.includes(value) ? "selected" : ""}`} key={value}><input type="checkbox" checked={allowed.includes(value)} onChange={() => toggle(value)} />{value}</label>)}</div>
        <label>默认强度<select aria-label="默认强度" value={effort} onChange={e => setEffort(e.target.value)}><option value="">模型默认（不指定强度）</option>{efforts.filter(v => allowed.includes(v)).map(value => <option key={value}>{value}</option>)}</select></label>
        <p className="hint">仅勾选该模型支持的档位。取消所有档位时只使用模型默认值。</p>
      </div>
      <div className="fields"><label>上下文窗口 · tokens<input type="number" min="1" step="1" placeholder="使用默认值" value={context} onChange={e => setContext(e.target.value)} /></label><label>最大输出 · tokens<input type="number" min="1" step="1" placeholder="使用默认值" value={maxTokens} onChange={e => setMaxTokens(e.target.value)} /></label></div>
      <div className="dialog-footer"><button type="button" onClick={cancel}>取消</button><button className="primary" type="submit">{busy ? "正在保存…" : "保存模型"}</button></div>
    </fieldset>
  </form></div>;
}
