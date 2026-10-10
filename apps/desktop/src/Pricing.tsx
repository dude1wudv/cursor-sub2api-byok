import { useCallback, useEffect, useState, type FormEvent } from "react";
import { request, type Model } from "./api";

type Price = { input_per_million: number; output_per_million: number; cache_read_per_million: number; cache_write_per_million: number };
type Settings = { currency: string; models: Record<string, Price> };
type Sync = { settings: { enabled: boolean; source_path: string }; checked_at_ms: number | null; synced_at_ms: number | null; stale: boolean; error: string | null; matched: Record<string, string>; unmatched: string[]; ambiguous: string[] };
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
  const [sync, setSync] = useState<Sync | null>(null);
  const [sourcePath, setSourcePath] = useState("");
  useEffect(() => {
    let active = true;
    setEstimate(null);
    void (async () => {
      const value = await request<Estimate>(`pricing/estimate?${query}`);
      const [prices, state] = await Promise.all([request<Settings>("pricing"), request<Sync>("pricing/sync")]);
      if (active) {
        setEstimate(value); setSync(state); setError("");
        if (!editing) { setSettings(prices); setSourcePath(state.settings.source_path); }
      }
    })().catch(e => { if (active) setError(String(e.message)); });
    return () => { active = false; };
  }, [query, revision, refreshKey, editing]);
  const changeSync = async (enabled: boolean, path?: string) => {
    if (!sync || saving) return;
    setSaving(true); setError(""); setNotice("");
    try {
      const state = await request<Sync>("pricing/sync", "PUT", { enabled, source_path: path ?? sync.settings.source_path });
      setSync(state); setSettings(await request<Settings>("pricing")); setRevision(value => value + 1);
      setNotice(enabled ? "已开启只读同步；估算与用量刷新时自动检查价格文件。" : "已关闭同步，保留最后有效单价，可手动编辑。");
    } catch (e) { setError(e instanceof Error ? e.message : "同步设置保存失败"); }
    finally { setSaving(false); }
  };
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
  const locked = saving || !sync || sync.settings.enabled;
  return <section className="card pricing-card">
    <div className="section-heading"><div><h2>Token 费用 · 本地估算</h2><p className="hint">按当前配置单价估算当前筛选区间，非 Sub2API 实际账单</p></div><button disabled={!settings || saving} onClick={() => { setEditing(!editing); setNotice(""); }}>{editing ? "收起配置" : "配置模型单价"}</button></div>
    {error && <div className="alert error" role="alert">{error}</div>}{notice && <div className="alert" role="status">{notice}</div>}
    {sync && <div className="pricing-sync">
      <label className="price-enable"><input type="checkbox" checked={sync.settings.enabled} disabled={saving} onChange={e => void changeSync(e.target.checked)} />自动同步 CC Switch 单价 · USD / 每百万 Token</label>
      <p className="hint" style={{ overflowWrap: "anywhere" }}>来源：{sync.settings.source_path}<br />上次成功读取：{sync.synced_at_ms ? new Date(sync.synced_at_ms).toLocaleString() : "尚未成功"} · 匹配 {Object.keys(sync.matched).length} 个模型{!sync.settings.enabled && " · 同步已关闭"}</p>
      {sync.settings.enabled && sync.stale && <div className="alert" role="status">{sync.error || "部分模型未匹配，已有价格继续保留；这些价格可能过期。"}
        {!!sync.unmatched.length && <p style={{ overflowWrap: "anywhere" }}>未匹配：{sync.unmatched.join("、")}</p>}
        {!!sync.ambiguous.length && <p style={{ overflowWrap: "anywhere" }}>匹配歧义：{sync.ambiguous.join("、")}</p>}
      </div>}
      <p className="hint">仅精确模型 ID 或唯一的去提供商前缀 ID 匹配，不推测其他型号。未匹配、歧义或源文件失效时保留旧价格；历史模型使用已保存单价。只读源文件，不调用模型。同步开启时锁定手动单价编辑。</p>
    </div>}
    <div className="estimate-total"><strong>{estimate ? `${estimate.currency} ${money(estimate.amount)}` : "—"}</strong><span>已配置单价部分的估算</span></div>
    <p className="hint">覆盖 {(estimate?.covered_tokens ?? 0).toLocaleString()} Token · 缺价 {(estimate?.unpriced_tokens ?? 0).toLocaleString()} Token · {estimate?.models.filter(model => model.amount === null).length ?? 0} 个模型缺价 · {estimate?.unknown_usage_calls ?? 0} 次请求用量不完整。仅计本机记录的 BYOK 用量，不含官方模型账单、平台加价、折扣或税费。</p>
    {!!estimate?.models.length && <div className="usage-table-scroll"><table className="usage-price-table"><thead><tr><th>模型</th><th>记录 Token</th><th>本地估算（{estimate.currency}）</th></tr></thead><tbody>{estimate.models.map((model, index) => <tr key={model.model_hash ?? `unknown-${index}`}><td>{model.display_name}{model.unknown_usage_calls > 0 && <small> · {model.unknown_usage_calls} 次用量不完整</small>}</td><td>{model.recorded_tokens.toLocaleString()}</td><td>{model.amount === null ? "缺少单价 · 未纳入金额" : money(model.amount)}</td></tr>)}</tbody></table></div>}
    {editing && settings && <form onSubmit={e => void save(e)}>
      {sync && <div><label>CC Switch 价格文件路径<input style={{ width: "100%" }} value={sourcePath} disabled={saving} onChange={e => setSourcePath(e.target.value)} /></label><button type="button" disabled={saving} onClick={() => void changeSync(sync.settings.enabled, sourcePath)}>保存路径并刷新</button></div>}
      <label className="currency-field">币种（所有模型统一）<input required pattern="[A-Z]{3}" maxLength={3} value={settings.currency} disabled={locked} onChange={e => setSettings({ ...settings, currency: e.target.value.toUpperCase() })} placeholder="USD / CNY" /></label>
      <p className="hint">每 1,000,000 Token 的价格；四项独立计费。输入已扣除缓存，避免重复计费。未启用的模型不估算，填写 0 表示该项免费。更换币种不会自动换算价格。</p>
      <div className="usage-table-scroll"><table className="usage-price-table"><thead><tr><th>启用 / 模型</th>{fields.map(([key, label]) => <th key={key}>{label}</th>)}</tr></thead><tbody>{[...editable].map(([hash, name]) => {
        const price = settings.models[hash];
        return <tr key={hash}><td><label className="price-enable"><input type="checkbox" checked={!!price} disabled={locked} onChange={e => { const next = { ...settings.models }; if (e.target.checked) next[hash] = { ...zeroPrice }; else delete next[hash]; setSettings({ ...settings, models: next }); }} />{name}</label>{sync?.matched[hash] && <small>CC Switch：{sync.matched[hash]}</small>}</td>{fields.map(([key, label]) => <td key={key}><input type="number" aria-label={`${name} ${label}每百万Token价格`} required min="0" max="1000000000" step="any" disabled={!price || locked} value={price && Number.isFinite(price[key]) ? price[key] : ""} onChange={e => setSettings({ ...settings, models: { ...settings.models, [hash]: { ...price, [key]: e.target.value === "" ? Number.NaN : Number(e.target.value) } } })} /></td>)}</tr>;
      })}</tbody></table></div>
      {!editable.size && <p className="hint">添加模型后可配置单价。</p>}
      <div className="form-footer"><span>仅保存在本机，不修改上游价格。</span><button type="submit" disabled={locked}>{saving ? "保存中…" : "保存单价"}</button></div>
    </form>}
  </section>;
}
