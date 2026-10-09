import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { HashRouter, Navigate, Route, Routes } from "react-router-dom";
import { defaultEffort, protocol, request, type Connection, type Model, type Status } from "./api";
import { ModelEditor } from "./ModelEditor";
import { ModelDiscovery } from "./ModelDiscovery";
import { CertificateSetup } from "./CertificateSetup";
import { Usage } from "./Usage";
const statusLabels = { disabled: "未接管", enabled: "接管中", degraded: "状态异常", recovery_required: "需要恢复" };
export function App() { return <HashRouter><Routes><Route path="/harness/cursor" element={<Controller />} /><Route path="*" element={<Navigate to="/harness/cursor" replace />} /></Routes></HashRouter>; }
function Controller() {
  const [certificateSetup, setCertificateSetup] = useState(false);
  const mainPanel = useRef<HTMLElement>(null);
  const [editingConnection, setEditingConnection] = useState(false);
  const enableAfterSetup = useRef(false);
  const operationPending = useRef(false);
  const [setupChecked, setSetupChecked] = useState(false);
  const [page, setPage] = useState<"models" | "usage" | "about">("models");
  useEffect(() => { mainPanel.current?.scrollTo({ top: 0 }); }, [page]);
  const [status, setStatus] = useState<Status | null>(null);
  const [connection, setConnection] = useState<Connection>({ base_url: "", has_api_key: false });
  const [models, setModels] = useState<Model[]>([]);
  const [base, setBase] = useState(""); const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false); const [error, setError] = useState(""); const [notice, setNotice] = useState("");
  const [editor, setEditor] = useState<Model | "new" | null>(null);
  const [discover, setDiscover] = useState(false);
  const [search, setSearch] = useState("");
  const [results, setResults] = useState<Record<string, { duration_ms: number; tokens_per_second: number; first_valid_response_ms: number | null; tokens_estimated: boolean }>>({});
  const refresh = useCallback(async () => {
    const [s, c, m] = await Promise.all([request<Status>("harness/cursor/status"), request<Connection>("sub2api/connection"), request<Model[]>("models")]);
    setStatus(s); setConnection(c); setModels(m); setBase(c.base_url);
  }, []);
  useEffect(() => { void refresh().catch(e => setError(String(e.message))); }, [refresh]);
  useEffect(() => {
    if (status && !setupChecked) {
      setSetupChecked(true);
      if (!status.certificate_consent && status.integration === "disabled") setCertificateSetup(true);
    }
  }, [status, setupChecked]);
  const perform = async (task: () => Promise<void>) => {
    if (operationPending.current) return;
    operationPending.current = true;
    setBusy(true); setError(""); setNotice("");
    try { await task(); await refresh(); } catch (e) { setError(e instanceof Error ? e.message : "操作失败"); await request<Status>("harness/cursor/status").then(setStatus).catch(() => {}); }
    finally { operationPending.current = false; setBusy(false); }
  };
  const toggleTakeover = () => void perform(async () => {
    // Read fresh state before deciding whether first-time consent is needed.
    const current = await request<Status>("harness/cursor/status");
    setStatus(current);
    if (current.restart_required) throw new Error("请保存工作并完全退出 Cursor，再更改接管状态。");
    if (current.integration === "disabled" && !current.certificate_consent) {
      enableAfterSetup.current = true;
      setCertificateSetup(true);
      return;
    }
    const next = await request<Status>("harness/cursor/enabled", "PUT", { enabled: current.integration !== "enabled" });
    setStatus(next);
    setNotice(next.integration === "enabled" ? "接管已开启，现在可以打开 Cursor。" : "Cursor 设置已恢复，证书保留供下次使用。");
  });
  const locked = busy || !status || status.integration !== "disabled";
  const saveConnection = (e: FormEvent) => { e.preventDefault(); void perform(async () => {
    await request("sub2api/connection", "PUT", { base_url: base, ...(key ? { api_key: key } : {}) });
    setKey(""); setEditingConnection(false); setNotice("连接已保存，所有模型已同步。");
  }); };
  const visible = models.filter(m => `${m.display_name} ${m.model_id}`.toLowerCase().includes(search.toLowerCase()));
  return <div className="app">
    <aside className="sidebar"><div className="brand"><span className="brand-mark">↗</span><div>Cursor <strong>BYOK</strong><small>SUB2API EDITION</small></div></div><div className="nav-label">工作台</div><nav>{([["models", "◈", "模型与连接"], ["usage", "▥", "用量统计"], ["about", "ⓘ", "关于"]] as const).map(([id, icon, label]) => <button key={id} className={page === id ? "active" : ""} onClick={() => setPage(id)}><span aria-hidden="true">{icon}</span>{label}</button>)}</nav><div className="sidebar-bottom"><span className="local-dot" />本地运行<small>由 <a href="https://microedulab.com/">MicroEduLab</a> 维护</small><small>v0.2.2 · Windows x64</small></div></aside>
    <div className="workspace"><header><div><span className="eyebrow">YOUR MODELS, IN CURSOR</span><h1>{page === "models" ? "模型与连接" : page === "usage" ? "用量概览" : "关于应用"}</h1><p className="page-description">{page === "models" ? "连接你的模型，让 Cursor 保持熟悉的工作方式。" : page === "usage" ? "看清每一次请求，掌握本机 Token 使用情况。" : "本地运行，配置由你掌控。"}</p></div><span className={`status ${status?.integration ?? ""}`}><i />{status ? statusLabels[status.integration] : "正在读取"}</span></header>
    <main ref={mainPanel}>
      {error && <div className="alert dismissible error" role="alert"><span>{error}</span><button aria-label="关闭错误提示" onClick={() => setError("")}>×</button></div>}{notice && <div className="alert dismissible" role="status"><span>{notice}</span><button aria-label="关闭操作提示" onClick={() => setNotice("")}>×</button></div>}
      {status?.recovery_error && <div className="alert error">{status.recovery_error}</div>}{!!status?.warnings.length && <div className="alert">已保留你修改的设置：{status.warnings.join("、")}</div>}
      {page === "models" && <section className={`card takeover ${status?.integration === "enabled" ? "is-active" : ""}`}><div className="takeover-copy"><span className="takeover-icon">⇄</span><div><div className="eyebrow">CONNECTION CONTROL</div><h2>Cursor 可逆接管</h2><p>{status?.restart_required ? "请先保存工作并完全退出 Cursor，再切换接管状态。" : "关闭和退出会恢复 Cursor 设置，专属证书保持安装。"}</p></div></div><div className="row-actions"><button disabled={busy} onClick={() => void perform(refresh)}>刷新状态</button>{status?.integration === "recovery_required" || status?.integration === "degraded" ? <button className="primary" disabled={busy} onClick={() => void perform(async () => { await request("harness/cursor/recover", "POST"); })}>恢复配置</button> : <button className="primary" disabled={busy || !status || (status.integration === "disabled" && (!models.length || !connection.has_api_key))} onClick={toggleTakeover}>{busy ? "正在处理…" : status?.integration === "enabled" ? "关闭并恢复" : "开启接管"}</button>}</div><div className="takeover-details"><span><i className={connection.has_api_key ? "ready" : ""} />连接{connection.has_api_key ? "已配置" : "待配置"}</span><span><i className={models.length ? "ready" : ""} />{models.length} 个模型</span><span><i className={status?.ca === "ready" ? "ready" : ""} />{status?.ca === "ready" ? "证书已就绪" : status?.certificate_consent ? "证书待安装 · 已授权" : "证书待授权"}</span></div></section>}
      {page === "models" && <>
        <section className="card connection"><div className="section-heading"><div><h2>Sub2API 连接</h2><p className="hint">一个连接，共享给所有模型</p></div><div className="row-actions"><span className={`badge ${connection.has_api_key ? "connected" : ""}`}>{connection.has_api_key ? "已保存连接" : "待配置"}</span>{connection.has_api_key && <button className="text-button" disabled={locked} aria-expanded={editingConnection} onClick={() => setEditingConnection(!editingConnection)}>{editingConnection ? "收起" : "编辑连接"}</button>}</div></div>
          {(!connection.has_api_key || editingConnection) ? <form onSubmit={saveConnection}><div className="fields"><label>Base URL<input type="url" required placeholder="https://your-sub2api.example/v1" value={base} disabled={locked} onChange={e => setBase(e.target.value)} /></label><label>API Key<input type="password" autoComplete="off" spellCheck={false} placeholder={connection.has_api_key ? "已加密保存 · 留空保持原 Key" : "输入专用 API Key"} value={key} required={!connection.has_api_key} disabled={locked} onChange={e => setKey(e.target.value)} /></label></div><div className="form-footer"><span>{locked && !busy ? "接管期间配置已锁定，请先关闭并恢复后编辑。" : "Key 由当前 Windows 用户加密保存在本机。"}</span><button disabled={locked} type="submit">保存连接</button></div></form> : <div className="connection-summary"><span className="connection-url">{connection.base_url}</span><span>API Key 已加密保存</span></div>}
        </section>
        <section className="model-section"><div className="section-heading"><div><h2>我的模型 <span className="count">{models.length}</span></h2><p className="hint">在 Cursor 原生模型选择器中使用</p></div><div className="row-actions"><button disabled={locked || !connection.has_api_key} onClick={() => setEditor("new")}>手动添加</button><button className="primary" disabled={locked || !connection.has_api_key} onClick={() => setDiscover(true)}>＋ 获取模型列表</button></div></div>
        {!!models.length && <div className="model-search"><input aria-label="搜索已添加模型" placeholder="搜索已添加模型…" value={search} onChange={e => setSearch(e.target.value)} /></div>}
        {!visible.length ? <div className="card empty"><span className="empty-symbol">◈</span><h3>{models.length ? "没有匹配的模型" : "连接你的第一个模型"}</h3><p>{models.length ? "试试其他搜索词。" : "保存连接后，获取可用模型列表并勾选添加。"}</p></div> : <div className="model-grid">{visible.map(model => <article className="card model-card" key={model.model_hash}><div className="model-card-heading"><span className={`model-icon ${model.type}`}>{model.type === "anthropic" ? "✳" : "◉"}</span><div className="model-copy"><h3>{model.display_name}</h3><span>{model.model_id}</span></div></div><div className="model-meta"><span className="badge">{protocol(model)}</span><span>{model.type === "anthropic" ? "Anthropic" : "OpenAI 兼容"}</span></div>
          <div className="model-config"><span>可选强度</span><div className="mini-chips">{model.allowed_reasoning_efforts.length ? model.allowed_reasoning_efforts.map(e => <span className={defaultEffort(model) === e ? "chosen" : ""} key={e}>{e}</span>) : <span>模型默认</span>}</div><span>默认强度</span><strong className="default-effort">{defaultEffort(model) ?? "模型默认"}<small>新会话默认</small></strong></div>
          <div className="test-result">{results[model.model_hash] ? <><span className="success-dot" />连接通过 <strong>{(results[model.model_hash].first_valid_response_ms ?? results[model.model_hash].duration_ms).toFixed(0)} ms</strong><span>·</span>{results[model.model_hash].tokens_per_second.toFixed(1)} tok/s{results[model.model_hash].tokens_estimated ? "（估算）" : ""}</> : <span>尚未测试 · 使用已保存连接</span>}</div>
          <div className="card-actions"><button disabled={busy} onClick={() => void perform(async () => { const result = await request<typeof results[string]>(`models/${model.model_hash}/test/${crypto.randomUUID()}`, "POST"); setResults(current => ({ ...current, [model.model_hash]: result })); })}>测试连接</button><button disabled={locked} onClick={() => setEditor(model)}>编辑配置</button><button className="danger text-button" disabled={locked} onClick={() => void perform(async () => { await request(`models/${model.model_hash}`, "DELETE"); })}>删除</button></div>
        </article>)}</div>}
        </section>
      </>}
      {page === "usage" && <Usage models={models} />}
      {page === "about" && <><section className="card about"><span className="eyebrow">CURSOR → SUB2API</span><h2>Cursor Sub2API BYOK <span className="badge">v0.2.2</span></h2><p>Windows x64 便携控制器。保留 Cursor 原生登录、Agent 与工具。</p><div className="about-links"><a href="https://microedulab.com/" rel="noreferrer">MicroEduLab ↗</a><a href="https://github.com/dude1wudv/cursor-sub2api-byok" rel="noreferrer">开发仓库 ↗</a><a href="https://github.com/leookun/cursor-byok" rel="noreferrer">上游 leookun/cursor-byok ↗</a></div><p className="hint">MIT · Copyright (c) 2026 leookun<br />数据使用 Windows CurrentUser DPAPI；其他用户或机器无法解密。</p></section><section className="card"><h2>目标路径与证书</h2><dl><dt>Cursor settings</dt><dd>{status?.settings_path || "—"}</dd><dt>本地代理</dt><dd>{status?.proxy_url || "未运行"}</dd><dt>CurrentUser Root CA</dt><dd>{status?.ca || "—"}</dd><dt>CA SHA-256</dt><dd>{status?.ca_sha256 || "首次开启接管时生成"}</dd></dl><div className="row-actions"><button disabled={busy || status?.integration !== "disabled"} onClick={() =>  { enableAfterSetup.current = false; setCertificateSetup(true); }}>查看使用说明 / 安装证书</button><button className="danger" disabled={busy || !status?.certificate_consent} onClick={() => void perform(async () => { await request("harness/cursor/ca/uninstall", "POST"); setNotice("专属证书已卸载，Cursor 设置已恢复。可退出后移除便携程序；模型配置和统计仍保留在数据目录。"); })}>卸载证书并恢复</button></div><p className="hint">日常关闭和退出保留证书；卸载时按同意记录精确清理。崩溃后重新打开程序恢复，或使用同一路径参数运行 --restore。</p></section></>}
      <footer><span>仅负责 Cursor → Sub2API</span><span>本地管理 · 127.0.0.1</span></footer>
    </main></div>
    {certificateSetup && <CertificateSetup cancel={() => { enableAfterSetup.current = false; setCertificateSetup(false); }} done={async () => {
      await refresh();
      setCertificateSetup(false);
      if (enableAfterSetup.current) {
        enableAfterSetup.current = false;
        await perform(async () => {
          const next = await request<Status>("harness/cursor/enabled", "PUT", { enabled: true });
          setStatus(next);
          setNotice("接管已开启，现在可以打开 Cursor。");
        });
      } else setNotice("证书已安装；日常开关和退出将保留证书，无需重复确认。");
    }} />}
    {editor && <ModelEditor key={typeof editor === "string" ? editor : editor.model_hash} model={editor} cancel={() => setEditor(null)} save={async input => { await request(editor === "new" ? "models" : `models/${editor.model_hash}`, editor === "new" ? "POST" : "PUT", editor === "new" ? { models: [input] } : input); setEditor(null); setError(""); await refresh(); }} />}
    {discover && <ModelDiscovery models={models} cancel={() => setDiscover(false)} save={async inputs => { await request("models", "POST", { models: inputs }); setDiscover(false); setError(""); setNotice(`已添加 ${inputs.length} 个模型。`); await refresh(); }} />}
  </div>;
}
