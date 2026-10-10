import { useCallback, useEffect, useState, type FormEvent } from "react";
import { request, type Model } from "./api";

type Price = { input_per_million: number; output_per_million: number; cache_read_per_million: number; cache_write_per_million: number };
type Settings = { currency: string; models: Record<string, Price> };
type Estimate = { currency: string; amount: number; covered_tokens: number; unpriced_tokens: number; unknown_usage_calls: number; models: { model_hash: string | null; display_name: string; calls: number; recorded_tokens: number; unknown_usage_calls: number; amount: number | null }[] };
const fields = [["input_per_million", "输入（非缓存）"], ["output_per_million", "输出"], ["cache_read_per_million", "缓存读取"], ["cache_write_per_million", "缓存写入"]] as const;
const zeroPrice: Price = { input_per_million: 0, output_per_million: 0, cache_read_per_million: 0, cache_write_per_million: 0 };
const money = (value: number) => value.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 6 });

export function Pricing({ models, query, refreshKey }: { models: Model[]; query: string; refreshKey: number }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [estimate, setEstimate] = useState<Estimate | null>(null);
  const [editing, setEditing] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [saving, setSaving] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    void request<Settings>("pricing").then(value => { if (active) setSettings(value); }).catch(e => { if (active) setError(String(e.message)); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    let active = true;
    setEstimate(null);
    void request<Estimate>(`pricing/estimate?${query}`).then(value => { if (active) { setEstimate(value); setError(""); } }).catch(e => { if (active) setError(String(e.message)); });
    return () => { active = false; };
  }, [query, revision, refreshKey]);
  const save = useCallback(async (e: FormEvent) => {
    e.preventDefault();
    if (!settings || saving) return;
    setSaving(true); setError(""); setNotice("");
    try { setSettings(await request<Settings>("pricing", "PUT", settings)); setRevision(value => value + 1); setEditing(false); setNotice("单价已保存，当前筛选区间已按新单价重新估算。"); }
    catch (e) { setError(e instanceof Error ? e.message : "单价保存失败"); }
    finally { setSaving(false); }
  }, [settings, saving]);
  const editable = new Map(models.map(model => [model.model_hash, model.display_name]));
  for (const model of estimate?.models ?? []) if (model.model_hash && !editable.has(model.model_hash)) editable.set(model.model_hash, `${model.display_name}（历史模型）`);
  for (const hash of Object.keys(settings?.models ?? {})) if (!editable.has(hash)) editable.set(hash, `历史配置 ${hash.slice(0, 12)}`);
  return <section className="card pricing-card">
    <div className="section-heading"><div><h2>Token 费用 · 本地估算</h2><p className="hint">按当前配置单价估算当前筛选区间，非 Sub2API 实际账单</p></div><button disabled={!settings || saving} onClick={() => { setEditing(!editing); setNotice(""); }}>{editing ? "收起配置" : "配置模型单价"}</button></div>
    {error && <div className="alert error" role="alert">{error}</div>}{notice && <div className="alert" role="status">{notice}</div>}
    <div className="estimate-total"><strong>{estimate ? `${estimate.currency} ${money(estimate.amount)}` : "—"}</strong><span>已配置单价部分的估算</span></div>
    <p className="hint">覆盖 {(estimate?.covered_tokens ?? 0).toLocaleString()} Token · 缺价 {(estimate?.unpriced_tokens ?? 0).toLocaleString()} Token · {estimate?.models.filter(model => model.amount === null).length ?? 0} 个模型缺价 · {estimate?.unknown_usage_calls ?? 0} 次请求用量不完整。仅计本机记录的 BYOK 用量，不含官方模型账单、平台加价、折扣或税费。</p>
    {!!estimate?.models.length && <div className="usage-table-scroll"><table className="usage-price-table"><thead><tr><th>模型</th><th>记录 Token</th><th>本地估算（{estimate.currency}）</th></tr></thead><tbody>{estimate.models.map((model, index) => <tr key={model.model_hash ?? `unknown-${index}`}><td>{model.display_name}{model.unknown_usage_calls > 0 && <small> · {model.unknown_usage_calls} 次用量不完整</small>}</td><td>{model.recorded_tokens.toLocaleString()}</td><td>{model.amount === null ? "缺少单价 · 未纳入金额" : money(model.amount)}</td></tr>)}</tbody></table></div>}
    {editing && settings && <form onSubmit={e => void save(e)}>
      <label className="currency-field">币种（所有模型统一）<input required pattern="[A-Z]{3}" maxLength={3} value={settings.currency} disabled={saving} onChange={e => setSettings({ ...settings, currency: e.target.value.toUpperCase() })} placeholder="USD / CNY" /></label>
      <p className="hint">每 1,000,000 Token 的价格；四项独立计费。输入已扣除缓存，避免重复计费。未启用的模型不估算，填写 0 表示该项免费。更换币种不会自动换算价格。</p>
      <div className="usage-table-scroll"><table className="usage-price-table"><thead><tr><th>启用 / 模型</th>{fields.map(([key, label]) => <th key={key}>{label}</th>)}</tr></thead><tbody>{[...editable].map(([hash, name]) => {
        const price = settings.models[hash];
        return <tr key={hash}><td><label className="price-enable"><input type="checkbox" checked={!!price} disabled={saving} onChange={e => { const next = { ...settings.models }; if (e.target.checked) next[hash] = { ...zeroPrice }; else delete next[hash]; setSettings({ ...settings, models: next }); }} />{name}</label></td>{fields.map(([key, label]) => <td key={key}><input type="number" aria-label={`${name} ${label}每百万Token价格`} required min="0" max="1000000000" step="any" disabled={!price || saving} value={price && Number.isFinite(price[key]) ? price[key] : ""} onChange={e => setSettings({ ...settings, models: { ...settings.models, [hash]: { ...price, [key]: e.target.value === "" ? Number.NaN : Number(e.target.value) } } })} /></td>)}</tr>;
      })}</tbody></table></div>
      {!editable.size && <p className="hint">添加模型后可配置单价。</p>}
      <div className="form-footer"><span>仅保存在本机，不修改上游价格。</span><button type="submit" disabled={saving}>{saving ? "保存中…" : "保存单价"}</button></div>
    </form>}
  </section>;
}
