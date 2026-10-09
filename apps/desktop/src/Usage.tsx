import { useCallback, useEffect, useState } from "react";
import { request, type Model, type Overview, type UsageBucket } from "./api";

export const number = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
const series = [ ["input_tokens", "输入（非缓存）", "#5c7bea"], ["cache_read_tokens", "缓存读取", "#43b4cc"], ["cache_write_tokens", "缓存写入", "#a58af2"], ["output_tokens", "输出", "#eead72"] ] as const;

export function Usage({ models }: { models: Model[] }) {
  const [days, setDays] = useState(7);
  const [model, setModel] = useState("");
  const [overview, setOverview] = useState<Overview | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const refresh = useCallback(async (signal?: AbortSignal) => {
    setLoading(true);
    try {
      const end = Date.now();
      const query = new URLSearchParams({ start_ms: String(end - days * 86400000), end_ms: String(end) });
      if (model) query.set("model_hashes", JSON.stringify([model]));
      const result = await request<Overview>(`overview?${query}`);
      if (!signal?.aborted) { setOverview(result); setError(""); }
    } catch (e) { if (!signal?.aborted) setError(e instanceof Error ? e.message : "统计读取失败"); }
    finally { if (!signal?.aborted) setLoading(false); }
  }, [days, model]);
  useEffect(() => {
    const controller = new AbortController();
    setOverview(null);
    void refresh(controller.signal);
    const timer = window.setInterval(() => { void refresh(controller.signal); }, 30000);
    return () => { controller.abort(); window.clearInterval(timer); };
  }, [refresh]);
  const m = overview?.metrics;
  const totalInput = m ? m.input_tokens + m.cache_read_tokens + m.cache_write_tokens : 0;
  const cacheReadRate = m && totalInput > 0 ? `${(m.cache_read_tokens / totalInput * 100).toFixed(1)}%` : "—";
  const data = overview?.token_usage_series ?? [];
  const total = (b: UsageBucket) => series.reduce((n, [key]) => n + b[key], 0);
  const max = Math.max(1, ...data.map(total));
  return <>
    <div className="section-heading"><div><h2>用量统计</h2><p className="hint">本机记录 · 每 30 秒刷新 · 不代表账户账单</p></div><div className="row-actions"><select aria-label="统计模型" value={model} onChange={e => setModel(e.target.value)}><option value="">全部模型</option>{models.map(m => <option value={m.model_hash} key={m.model_hash}>{m.display_name}</option>)}</select><select aria-label="统计时间范围" value={days} onChange={e => setDays(Number(e.target.value))}><option value={1}>最近 24 小时</option><option value={7}>最近 7 天</option><option value={30}>最近 30 天</option></select><button disabled={loading} onClick={() => void refresh()}>刷新</button></div></div>
    {error && <div className="alert error" role="alert">{error}</div>}
    <div className="metric-grid">{[
      ["总 Token", m?.token_usage, "输入 + 缓存 + 输出"], ["请求次数", m?.llm_calls, `${m?.successful_calls ?? 0} 成功 · ${m?.failed_calls ?? 0} 未成功`], ["输入 Token", m?.input_tokens, "不含缓存部分"], ["输出 Token", m?.output_tokens, "模型生成用量"],
      ["缓存读取 Token", m?.cache_read_tokens, "复用已有输入缓存"], ["缓存读取率", cacheReadRate, "缓存读取 ÷ 全部输入 Token"],
    ].map(([label, value, note]) => <div className="metric card" key={String(label)}><span>{label}</span><strong title={typeof value === "number" ? value.toLocaleString() : ""}>{typeof value === "number" ? number(value) : value ?? "—"}</strong><small>{note}</small></div>)}</div>
    <section className="card chart-card"><div className="section-heading"><div><h2>Token 使用趋势</h2><p className="hint">悬停或聚焦柱形查看明细</p></div><span className="badge">{overview?.token_usage_granularity === "day" ? "按天" : "按小时"}</span></div>
      <div className="legend">{series.map(([, label, color]) => <span key={label}><i style={{ background: color }} />{label}</span>)}</div>
      <div className="chart" aria-label="Token 用量柱形图">{data.map((b, index) => {
        const date = new Date(b.bucket_start_ms);
        const caption = overview?.token_usage_granularity === "day" ? `${date.getMonth() + 1}/${date.getDate()}` : `${date.getHours()}:00`;
        const detail = `${date.toLocaleString()}\n${series.map(([key, label]) => `${label}：${b[key].toLocaleString()}`).join("\n")}`;
        return <div className="chart-column" key={b.bucket_start_ms} tabIndex={0} aria-label={detail} title={detail}><div className="bar-track"><div className="bar-stack" style={{ height: `${total(b) / max * 100}%` }}>{series.map(([key, , color]) => b[key] > 0 && <i key={key} style={{ background: color, flexGrow: b[key] }} />)}</div></div><span>{index % Math.max(1, Math.ceil(data.length / 12)) === 0 ? caption : ""}</span></div>;
      })}</div>
      {!loading && data.every(b => total(b) === 0) && <p className="chart-empty">这个时间范围暂无 Token 用量，开始使用后将在这里显示。</p>}
      <div className="cache-summary"><span>缓存写入 <strong>{m ? number(m.cache_write_tokens) : "—"}</strong></span><span>读取率按当前筛选汇总计算，全部输入包含缓存读取与写入，不含输出。</span></div>
    </section>
  </>;
}
