import { useCallback, useEffect, useState, type FormEvent } from "react";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { request, type Connection, type Model, type Status } from "./api";
const statusLabels = { disabled: "未接管", enabled: "接管中", degraded: "状态异常", recovery_required: "需要恢复" };
export function App() { return <HashRouter><Routes><Route path="/harness/cursor" element={<Controller />} /><Route path="*" element={<Navigate to="/harness/cursor" replace />} /></Routes></HashRouter>; }
function Controller() {
  const [status, setStatus] = useState<Status | null>(null);
  const [connection, setConnection] = useState<Connection>({ base_url: "", has_api_key: false });
  const [models, setModels] = useState<Model[]>([]);
  const [base, setBase] = useState(""); const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false); const [error, setError] = useState(""); const [notice, setNotice] = useState("");
  const [editor, setEditor] = useState<Model | "new" | null>(null);
  const refresh = useCallback(async () => {
    const [s, c, m] = await Promise.all([request<Status>("harness/cursor/status"), request<Connection>("sub2api/connection"), request<Model[]>("models")]);
    setStatus(s); setConnection(c); setModels(m); setBase(c.base_url);
  }, []);
  useEffect(() => { void refresh().catch(e => setError(String(e.message))); }, [refresh]);
  const perform = async (task: () => Promise<void>) => {
    setBusy(true); setError(""); setNotice("");
    try { await task(); await refresh(); } catch (e) { setError(e instanceof Error ? e.message : "操作失败"); await request<Status>("harness/cursor/status").then(setStatus).catch(() => {}); }
    finally { setBusy(false); }
  };
  const locked = busy || !status || status.integration !== "disabled";
  const saveConnection = (e: FormEvent) => { e.preventDefault(); void perform(async () => {
    await request("sub2api/connection", "PUT", { base_url: base, ...(key ? { api_key: key } : {}) });
    setKey(""); setNotice("连接已保存，所有模型已同步。");
  }); };
  return <div className="app">
    <header><div><span className="eyebrow">SUB2API · LOCAL CONTROLLER</span><h1>Cursor 接管</h1><p>一个连接，多个模型。保留 Cursor 原生登录与工具。</p></div><span className={`status ${status?.integration ?? ""}`}>{status ? statusLabels[status.integration] : "正在读取"}</span></header>
    <main>
      {error && <div className="alert error" role="alert">{error}</div>}
      {notice && <div className="alert" role="status">{notice}</div>}
      {status?.recovery_error && <div className="alert error">{status.recovery_error}</div>}
      {!!status?.warnings.length && <div className="alert">已保留你修改的设置：{status.warnings.join("、")}</div>}
      <section className="card connection"><div className="section-heading"><h2>Sub2API 连接</h2><span>共享给下方所有模型</span></div>
        <form onSubmit={saveConnection}><div className="fields"><label>Base URL<input type="url" required placeholder="https://your-sub2api.example/v1" value={base} disabled={locked} onChange={e => setBase(e.target.value)} /></label><label>API Key<input type="password" autoComplete="off" spellCheck={false} placeholder={connection.has_api_key ? "已保存；留空保持原 Key" : "输入专用 API Key"} value={key} required={!connection.has_api_key} disabled={locked} onChange={e => setKey(e.target.value)} /></label></div><div className="form-footer"><span>Key 使用当前 Windows 用户加密保存。</span><button disabled={locked} type="submit">保存连接</button></div></form>
      </section>
      <section className="card"><div className="section-heading"><h2>模型 <small>{models.length}</small></h2><button disabled={locked || !connection.has_api_key} onClick={() => setEditor("new")}>＋ 添加模型</button></div>
        {models.length === 0 ? <div className="empty">保存连接后添加 GPT 或 Claude 模型。<br /><span>模型 ID 应与 Sub2API 中的可用模型一致。</span></div> : <ul className="models">{models.map(model => <li key={model.model_hash}><span className="model-icon">{model.type === "anthropic" ? "C" : "G"}</span><div className="model-copy"><strong>{model.display_name}</strong><span>{model.model_id} · {model.type === "anthropic" ? "Messages" : model.openai_endpoint.includes("responses") ? "Responses" : "Chat Completions"}</span></div><div className="row-actions"><button disabled={busy} onClick={() => void perform(async () => { const result = await request<{duration_ms: number}>(`models/${model.model_hash}/test/${crypto.randomUUID()}`, "POST"); setNotice(`${model.display_name} 连接测试通过（${result.duration_ms} ms）。`); })}>测试</button><button disabled={locked} onClick={() => setEditor(model)}>编辑</button><button className="danger" disabled={locked} onClick={() => void perform(async () => { await request(`models/${model.model_hash}`, "DELETE"); })}>删除</button></div></li>)}</ul>}
      </section>
      <section className="card takeover"><div><h2>可逆接管</h2><p>{status?.restart_required ? "请先保存工作并完全退出 Cursor，再开启或关闭接管。" : "切换后手动打开 Cursor。退出本程序会恢复配置。"}</p></div><div className="row-actions"><button disabled={busy} onClick={() => void perform(refresh)}>刷新状态</button>{status?.integration === "recovery_required" || status?.integration === "degraded" ? <button className="primary" disabled={busy} onClick={() => void perform(async () => { await request("harness/cursor/recover", "POST"); })}>恢复配置</button> : <button className="primary" disabled={busy || !status || (status.integration === "disabled" && (!models.length || !connection.has_api_key))} onClick={() => void perform(async () => { await request("harness/cursor/enabled", "PUT", { enabled: status?.integration !== "enabled" }); })}>{status?.integration === "enabled" ? "关闭并恢复" : "开启接管"}</button>}</div></section>
      <details className="details"><summary>目标路径与证书</summary><dl><dt>Cursor settings</dt><dd>{status?.settings_path || "—"}</dd><dt>本地代理</dt><dd>{status?.proxy_url || "未运行"}</dd><dt>CurrentUser Root CA</dt><dd>{status?.ca || "—"}</dd><dt>CA SHA-256</dt><dd>{status?.ca_sha256 || "首次开启接管时生成"}</dd></dl><p>只清理本次接管安装的专属证书。崩溃后可重新打开程序恢复，或使用同一路径参数运行 --restore。</p></details>
      <details className="details"><summary>关于</summary><p>Cursor Sub2API BYOK 0.1.0 · Windows x64 · MIT</p><p>由 <a href="https://microedulab.com/" rel="noreferrer">MicroEduLab</a> 开发与维护 · <a href="https://github.com/dude1wudv/cursor-sub2api-byok" rel="noreferrer">开发仓库</a></p><p><a href="https://github.com/leookun/cursor-byok" rel="noreferrer">上游来源 leookun/cursor-byok</a> · Copyright (c) 2026 leookun</p><p>数据目录使用 Windows CurrentUser DPAPI；复制到其他用户或机器后不能解密。</p></details>
    </main>
    <footer><span>只负责 Cursor → Sub2API</span><span>本地管理 · 127.0.0.1</span></footer>
    {editor && <ModelEditor key={typeof editor === "string" ? editor : editor.model_hash} model={editor} disabled={busy} cancel={() => setEditor(null)} save={input => perform(async () => { await request(editor === "new" ? "models" : `models/${editor.model_hash}`, editor === "new" ? "POST" : "PUT", editor === "new" ? { models: [input] } : input); setEditor(null); })} />}
  </div>;
}
function ModelEditor({model, disabled, cancel, save}: {model: Model | "new"; disabled: boolean; cancel: () => void; save: (input: unknown) => Promise<void>}) {
  const old = model === "new" ? null : model;
  const [name, setName] = useState(old?.display_name ?? ""); const [id, setId] = useState(old?.model_id ?? "");
  const [type, setType] = useState(old?.type ?? "openai"); const [endpoint, setEndpoint] = useState(old?.openai_endpoint || "/v1/responses");
  const [effort, setEffort] = useState(old?.type === "anthropic" ? old.anthropic_thinking_effort ?? "" : old?.reasoning_effort ?? "");
  const [context, setContext] = useState(old?.context_window_tokens?.toString() ?? ""); const [maxTokens, setMaxTokens] = useState(old?.max_completion_tokens?.toString() ?? "");
  return <div className="overlay"><form className="dialog" role="dialog" aria-modal="true" aria-labelledby="editor-title" onSubmit={e => { e.preventDefault(); void save({display_name: name, model_id: id, type, openai_endpoint: endpoint, reasoning_effort: effort || null, context_window_tokens: context ? Number(context) : null, max_completion_tokens: maxTokens ? Number(maxTokens) : null}); }}><h2 id="editor-title">{old ? "编辑模型" : "添加模型"}</h2><fieldset disabled={disabled}><label>显示名称<input autoFocus required value={name} onChange={e => setName(e.target.value)} /></label><label>模型 ID<input required value={id} onChange={e => setId(e.target.value)} placeholder="与 Sub2API 模型 ID 完全一致" /></label><div className="fields"><label>协议<select value={type} onChange={e => setType(e.target.value as Model["type"])}><option value="openai">OpenAI / GPT</option><option value="anthropic">Anthropic / Claude</option></select></label><label>{type === "openai" ? "Endpoint" : "Endpoint（固定）"}<select disabled={type === "anthropic"} value={type === "anthropic" ? "/v1/messages" : endpoint} onChange={e => setEndpoint(e.target.value)}>{type === "anthropic" ? <option value="/v1/messages">Messages</option> : <><option value="/v1/responses">Responses（默认）</option><option value="/v1/chat/completions">Chat Completions</option></>}</select></label></div><label>Reasoning effort<select value={effort} onChange={e => setEffort(e.target.value)}><option value="">不指定，使用模型默认值</option>{["low","medium","high","xhigh","max"].map(v => <option key={v}>{v}</option>)}</select></label><div className="fields"><label>上下文窗口（可选）<input type="number" min="1" value={context} onChange={e => setContext(e.target.value)} /></label><label>最大输出 tokens（可选）<input type="number" min="1" value={maxTokens} onChange={e => setMaxTokens(e.target.value)} /></label></div><div className="dialog-footer"><button type="button" onClick={cancel}>取消</button><button className="primary" type="submit">保存模型</button></div></fieldset></form></div>;
}
