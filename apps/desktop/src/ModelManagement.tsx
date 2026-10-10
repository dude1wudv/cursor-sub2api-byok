import { useEffect, useRef, useState } from "react";
import { defaultEffort, protocol, request, type Model, type ModelInput } from "./api";
import "./model-management.scss";

type ConnectivityResult = { duration_ms: number; first_valid_response_ms: number | null; tokens_per_second: number; tokens_estimated: boolean };
type TestState = { status: "queued" | "running" | "cancelling" | "cancelled" | "success" | "failed"; result?: ConnectivityResult; error?: string };
type TestJob = { hash: string; id: string; started: boolean; cancelled: boolean };
type Props = {
  models: Model[]; locked: boolean; hasConnection: boolean;
  onRefresh: () => Promise<void>; onError: (message: string) => void;
  onEdit: (model: Model | "new") => void; onDiscover: () => void;
};
const message = (error: unknown) => error instanceof Error ? error.message : "操作失败";
const testPath = (job: TestJob) => `models/${encodeURIComponent(job.hash)}/test/${job.id}`;
const groupName = (model: Model) => model.group_name || "未分组";
function copyInput(model: Model, displayName: string, sortOrder: number): ModelInput {
  // Explicit public fields only. The server resolves the shared DPAPI connection.
  return { display_name: displayName, model_id: model.model_id, type: model.type,
    group_name: model.group_name, sort_order: sortOrder, openai_endpoint: model.openai_endpoint,
    reasoning_effort: defaultEffort(model), allowed_reasoning_efforts: model.allowed_reasoning_efforts,
    context_window_tokens: model.context_window_tokens, max_completion_tokens: model.max_completion_tokens,
    thinking_budget_tokens: model.thinking_budget_tokens };
}

export function ModelManagement({ models, locked, hasConnection, onRefresh, onError, onEdit, onDiscover }: Props) {
  const [search, setSearch] = useState("");
  const [group, setGroup] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [tests, setTests] = useState<Record<string, TestState>>({});
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const jobs = useRef(new Map<string, TestJob>());
  const mutationPending = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    const active = jobs.current;
    return () => {
      mounted.current = false;
      for (const job of active.values()) {
        job.cancelled = true;
        if (job.started) void request(testPath(job), "DELETE").catch(() => {});
      }
    };
  }, []);
  const updateTest = (hash: string, state: TestState) => {
    if (mounted.current) setTests(current => ({ ...current, [hash]: state }));
  };
  const mutate = async (operation: () => Promise<unknown>) => {
    if (locked || mutationPending.current || jobs.current.size) return;
    mutationPending.current = true; setSaving(true);
    try { await operation(); await onRefresh(); } catch (error) { onError(message(error)); }
    finally { mutationPending.current = false; if (mounted.current) setSaving(false); }
  };
  const startTests = async (targets: Model[]) => {
    if (!targets.length || jobs.current.size || mutationPending.current || !hasConnection) return;
    const queue = targets.map(model => ({ hash: model.model_hash, id: crypto.randomUUID(), started: false, cancelled: false }));
    for (const job of queue) { jobs.current.set(job.hash, job); updateTest(job.hash, { status: "queued" }); }
    setTesting(true);
    // Three bounded workers; cancelling queued entries never sends their POST.
    const worker = async () => {
      for (let job = queue.shift(); job; job = queue.shift()) {
        if (job.cancelled) { jobs.current.delete(job.hash); continue; }
        job.started = true; updateTest(job.hash, { status: "running" });
        try {
          const result = await request<ConnectivityResult>(testPath(job), "POST");
          updateTest(job.hash, { status: "success", result });
        } catch (error) {
          const detail = message(error);
          updateTest(job.hash, detail.includes("run was cancelled") ? { status: "cancelled" } : { status: "failed", error: detail });
        } finally { jobs.current.delete(job.hash); }
      }
    };
    await Promise.all([worker(), worker(), worker()]);
    if (mounted.current) setTesting(false);
  };
  const cancelTest = async (job: TestJob) => {
    if (job.cancelled) return;
    job.cancelled = true;
    if (!job.started) { updateTest(job.hash, { status: "cancelled" }); return; }
    updateTest(job.hash, { status: "cancelling" });
    try { await request(testPath(job), "DELETE"); }
    catch (error) {
      job.cancelled = false;
      if (jobs.current.has(job.hash)) updateTest(job.hash, { status: "running" });
      onError(`取消请求未确认：${message(error)}`);
    }
  };
  const copy = (model: Model) => mutate(async () => {
    let suffix = 1;
    let name = `${model.display_name} 副本`;
    while (models.some(item => item.display_name === name)) name = `${model.display_name} 副本 ${++suffix}`;
    await request("models", "POST", { models: [copyInput(model, name, Math.max(0, ...models.map(item => item.sort_order)) + 1)] });
  });
  const move = (model: Model, direction: -1 | 1) => mutate(async () => {
    const siblings = models.filter(item => groupName(item) === groupName(model));
    const sibling = siblings[siblings.indexOf(model) + direction];
    if (!sibling) return;
    const hashes = models.map(item => item.model_hash);
    const from = hashes.indexOf(model.model_hash); const to = hashes.indexOf(sibling.model_hash);
    [hashes[from], hashes[to]] = [hashes[to], hashes[from]];
    await request("models/order", "PUT", { model_hashes: hashes });
  });
  const groups = [...new Set(models.map(groupName))];
  const visible = models.filter(model => (!group || groupName(model) === group) && `${model.display_name} ${model.model_id}`.toLowerCase().includes(search.toLowerCase()));
  const selectedModels = models.filter(model => selected.includes(model.model_hash));
  const disableEdits = locked || saving || testing;
  return <section className="model-section model-management">
    <div className="section-heading"><div><h2>我的模型 <span className="count">{models.length}</span></h2><p className="hint">在 Cursor 原生模型选择器中使用</p></div><div className="row-actions"><button disabled={disableEdits || !hasConnection} onClick={() => onEdit("new")}>手动添加</button><button className="primary" disabled={disableEdits || !hasConnection} onClick={onDiscover}>＋ 获取模型列表</button></div></div>
    {!!models.length && <><div className="model-management-filters"><input aria-label="搜索已添加模型" placeholder="搜索已添加模型…" value={search} onChange={event => setSearch(event.target.value)} /><select aria-label="筛选模型分组" value={group} onChange={event => setGroup(event.target.value)}><option value="">全部分组</option>{groups.map(name => <option key={name}>{name}</option>)}</select></div>
      <div className="model-batch-actions"><label className="model-select"><input type="checkbox" checked={!!visible.length && visible.every(model => selected.includes(model.model_hash))} onChange={event => setSelected(current => event.target.checked ? [...new Set([...current, ...visible.map(model => model.model_hash)])] : current.filter(hash => !visible.some(model => model.model_hash === hash)))} />选择当前显示</label><span>已选 {selectedModels.length} 项</span><button disabled={testing || saving || !hasConnection || !selectedModels.length} onClick={() => void startTests(selectedModels)}>测试所选 · 产生用量</button>{testing && <button className="danger" onClick={() => { for (const job of jobs.current.values()) void cancelTest(job); }}>取消全部测试</button>}</div><p className="hint">测试使用固定合成提示词，每模型一次请求，最多 256 输出 Token；同时最多 3 项。结果仅保留在当前页面。</p></>}
    {!visible.length ? <div className="card empty"><span className="empty-symbol">◈</span><h3>{models.length ? "没有匹配的模型" : "连接你的第一个模型"}</h3><p>{models.length ? "试试其他搜索词或分组。" : "保存连接后，获取可用模型列表并勾选添加。"}</p></div> : groups.filter(name => visible.some(model => groupName(model) === name)).map(name => <div className="model-group" key={name}><h3 className="model-group-title">{name}<span>{visible.filter(model => groupName(model) === name).length}</span></h3><div className="model-grid">{visible.filter(model => groupName(model) === name).map(model => {
      const state = tests[model.model_hash]; const job = jobs.current.get(model.model_hash);
      const siblings = models.filter(item => groupName(item) === name); const index = siblings.indexOf(model);
      return <article className="card model-card" key={model.model_hash}>
        <div className="model-card-heading"><input className="model-checkbox" type="checkbox" aria-label={`选择 ${model.display_name}`} checked={selected.includes(model.model_hash)} onChange={event => setSelected(current => event.target.checked ? [...current, model.model_hash] : current.filter(hash => hash !== model.model_hash))} /><span className={`model-icon ${model.type}`}>{model.type === "anthropic" ? "✳" : "◉"}</span><div className="model-copy"><h3>{model.display_name}</h3><span>{model.model_id}</span></div></div>
        <div className="model-meta"><span className="badge">{protocol(model)}</span><span>{model.type === "anthropic" ? "Anthropic" : "OpenAI 兼容"}</span></div>
        <div className="model-config"><span>可选强度</span><div className="mini-chips">{model.allowed_reasoning_efforts.length ? model.allowed_reasoning_efforts.map(effort => <span className={defaultEffort(model) === effort ? "chosen" : ""} key={effort}>{effort}</span>) : <span>模型默认</span>}</div><span>默认强度</span><strong className="default-effort">{defaultEffort(model) ?? "模型默认"}<small>新会话默认</small></strong></div>
        <div className={`test-result test-${state?.status ?? "idle"}`} role="status">{state?.status === "success" && state.result ? <><span className="success-dot" />连接通过 <strong>{(state.result.first_valid_response_ms ?? state.result.duration_ms).toFixed(0)} ms</strong><span>· {state.result.tokens_per_second.toFixed(1)} tok/s{state.result.tokens_estimated ? "（估算）" : ""}</span></> : state?.status === "failed" ? <span>失败：{state.error}</span> : <span>{state?.status === "queued" ? "等待测试" : state?.status === "running" ? "正在测试…" : state?.status === "cancelling" ? "正在停止请求…" : state?.status === "cancelled" ? "已取消" : "尚未测试 · 使用已保存连接"}</span>}</div>
        <div className="card-actions">{job && (state?.status === "running" || state?.status === "queued" || state?.status === "cancelling") ? <button disabled={state.status === "cancelling"} onClick={() => void cancelTest(job)}>取消测试</button> : <button disabled={testing || saving || !hasConnection} onClick={() => void startTests([model])}>测试 · 产生用量</button>}<button disabled={disableEdits} onClick={() => onEdit(model)}>编辑</button><button disabled={disableEdits} onClick={() => void copy(model)}>复制</button><button className="danger text-button" disabled={disableEdits} onClick={() => void mutate(() => request(`models/${model.model_hash}`, "DELETE"))}>删除</button></div>
        <div className="model-order"><span>组内顺序 {index + 1}</span><button disabled={disableEdits || index === 0} aria-label={`上移 ${model.display_name}`} onClick={() => void move(model, -1)}>↑</button><button disabled={disableEdits || index === siblings.length - 1} aria-label={`下移 ${model.display_name}`} onClick={() => void move(model, 1)}>↓</button></div>
      </article>;
    })}</div></div>)}
  </section>;
}
