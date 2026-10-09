import { useEffect, useState } from "react";
import { efforts, request, type Model, type ModelInput } from "./api";

export function ModelDiscovery({ models, cancel, save }: { models: Model[]; cancel: () => void; save: (models: ModelInput[]) => Promise<void> }) {
  const [ids, setIds] = useState<string[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [types, setTypes] = useState<Record<string, Model["type"]>>({});
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const existing = new Set(models.map(m => m.model_id));
  const load = async () => {
    setLoading(true); setError("");
    try { setIds((await request<{ id: string }[]>("sub2api/models")).map(m => m.id)); }
    catch (e) { setError(e instanceof Error ? e.message : "读取失败"); }
    finally { setLoading(false); }
  };
  useEffect(() => { void load(); }, []);
  const visible = ids.filter(id => id.toLowerCase().includes(query.toLowerCase()));
  const modelType = (id: string) => types[id] ?? (/claude/i.test(id) ? "anthropic" : "openai");
  const toggle = (id: string) => setSelected(current => { const next = new Set(current); if (next.has(id)) next.delete(id); else next.add(id); return next; });
  return <div className="overlay"><div className="dialog discovery" role="dialog" aria-modal="true" aria-labelledby="discovery-title">
    <div className="section-heading"><div><span className="eyebrow">SUB2API MODEL CATALOG</span><h2 id="discovery-title">选择可用模型</h2></div><button className="icon-button" aria-label="关闭" disabled={saving} onClick={cancel}>×</button></div>
    <p className="hint">通过已保存的连接请求 /v1/models。协议按名称预选，添加前可调整；强度与默认值可在模型卡片中编辑。</p>
    {error && <div className="alert error" role="alert">{error}</div>}
    <div className="discovery-toolbar"><input aria-label="搜索模型" placeholder="搜索模型名称…" value={query} onChange={e => setQuery(e.target.value)} /><button disabled={loading || saving} onClick={() => void load()}>重新获取</button></div>
    <div className="section-heading"><span>{loading ? "正在读取…" : `${visible.length} 个模型 · 已选 ${selected.size} 个`}</span><button className="text-button" disabled={loading || saving} onClick={() => setSelected(current => new Set([...current, ...visible.filter(id => !existing.has(id))]))}>选择搜索结果</button><button className="text-button" disabled={saving} onClick={() => setSelected(new Set())}>清空</button></div>
    <div className="discovery-list">{!loading && !visible.length && <div className="empty">{error ? "请检查连接后重新获取。" : "没有匹配的可用模型。"}</div>}{visible.map(id => <div className="discovery-row" key={id}><label><input type="checkbox" disabled={saving || existing.has(id)} checked={existing.has(id) || selected.has(id)} onChange={() => toggle(id)} /><span>{id}{existing.has(id) && <small>已添加</small>}</span></label><select aria-label={`${id} 协议`} disabled={saving || existing.has(id)} value={modelType(id)} onChange={e => setTypes(current => ({ ...current, [id]: e.target.value as Model["type"] }))}><option value="openai">OpenAI · Responses</option><option value="anthropic">Anthropic · Messages</option></select></div>)}</div>
    <div className="dialog-footer"><button disabled={saving} onClick={cancel}>取消</button><button className="primary" disabled={loading || saving || !selected.size} onClick={async () => {
      setSaving(true); setError("");
      try { await save([...selected].map(id => ({ display_name: id, model_id: id, type: modelType(id), openai_endpoint: "/v1/responses", reasoning_effort: null, allowed_reasoning_efforts: efforts }))); }
      catch (e) { setError(e instanceof Error ? e.message : "添加失败"); }
      finally { setSaving(false); }
    }}>{saving ? "正在添加…" : `添加所选 ${selected.size} 个模型`}</button></div>
  </div></div>;
}
