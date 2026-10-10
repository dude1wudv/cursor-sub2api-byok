import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { request, type Model, type Overview, type UsageBucket } from "./api";

import { Pricing } from "./Pricing";
import "./usage.css";

type CalendarDay = UsageBucket & { date: string; end_ms: number; calls: number };
type UsageOverview = Overview & { calendar: CalendarDay[]; timezone: string };
const calendarColors = ["#eff0f9", "#d6d5fa", "#aaa5ed", "#8176d8", "#5c50b7"];
const localDate = (date: Date) => `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
const initialDate = (daysAgo: number) => { const date = new Date(); date.setDate(date.getDate() - daysAgo); return localDate(date); };

export const number = (value: number) => new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
const series = [ ["input_tokens", "输入（非缓存）", "#5c7bea"], ["cache_read_tokens", "缓存读取", "#43b4cc"], ["cache_write_tokens", "缓存写入", "#a58af2"], ["output_tokens", "输出", "#eead72"] ] as const;

export function Usage({ models }: { models: Model[] }) {
  const [days, setDays] = useState("7");
  const [selectedModels, setSelectedModels] = useState<string[]>([]);
  const [startDate, setStartDate] = useState(() => initialDate(6));
  const [endDate, setEndDate] = useState(() => initialDate(0));
  const [tick, setTick] = useState(0);
  const [overview, setOverview] = useState<UsageOverview | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const sequence = useRef(0);
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  const range = useMemo(() => {
    void tick;
    let start: number; let end: number;
    if (days === "custom") {
      start = new Date(`${startDate}T00:00:00`).getTime();
      const endDay = new Date(`${endDate}T00:00:00`);
      endDay.setDate(endDay.getDate() + 1);
      end = endDay.getTime();
    } else {
      const startDay = new Date();
      startDay.setHours(0, 0, 0, 0); startDay.setDate(startDay.getDate() - Number(days) + 1);
      start = startDay.getTime(); end = Date.now();
    }
    if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start || end - start > 3660 * 86400000) return null;
    const query = new URLSearchParams({ start_ms: String(start), end_ms: String(end), timezone });
    if (selectedModels.length) query.set("model_hashes", JSON.stringify(selectedModels));
    return query.toString();
  }, [days, startDate, endDate, selectedModels, tick, timezone]);
  const refresh = useCallback(async (signal?: AbortSignal) => {
    const current = ++sequence.current;
    if (!range) { setError("请选择有效日期，起始日期不能晚于结束日期，范围最多 3660 天。"); setOverview(null); setLoading(false); return; }
    setLoading(true);
    try {
      const result = await request<UsageOverview>(`overview?${range}`);
      if (!signal?.aborted && sequence.current === current) { setOverview(result); setError(""); }
    } catch (e) { if (!signal?.aborted && sequence.current === current) { setOverview(null); setError(e instanceof Error ? e.message : "统计读取失败"); } }
    finally { if (!signal?.aborted && sequence.current === current) setLoading(false); }
  }, [range]);
  useEffect(() => {
    const controller = new AbortController(); setOverview(null);
    void refresh(controller.signal);
    return () => { controller.abort(); };
  }, [refresh, tick]);
  useEffect(() => { const timer = window.setInterval(() => setTick(value => value + 1), 30000); return () => window.clearInterval(timer); }, []);
  const m = overview?.metrics;
  const totalInput = m ? m.input_tokens + m.cache_read_tokens + m.cache_write_tokens : 0;
  const cacheReadRate = m && totalInput > 0 ? `${(m.cache_read_tokens / totalInput * 100).toFixed(1)}%` : "—";
  const data = overview?.token_usage_series ?? [];
  const total = (b: UsageBucket) => series.reduce((n, [key]) => n + b[key], 0);
  const max = Math.max(1, ...data.map(total));
  const calendarMax = Math.max(1, ...(overview?.calendar.map(total) ?? []));
  return <>
    <div className="section-heading"><div><h2>用量统计</h2><p className="hint">本机 BYOK 记录 · 每 30 秒刷新 · 时区 {timezone}</p></div><div className="row-actions"><select aria-label="统计时间范围" value={days} onChange={e => setDays(e.target.value)}><option value="1">今天</option><option value="7">最近 7 天</option><option value="30">最近 30 天</option><option value="365">最近一年</option><option value="custom">自定义日期</option></select><button disabled={loading} onClick={() => setTick(value => value + 1)}>刷新</button></div></div>
    <section className="card usage-filter-card">
      {days === "custom" && <div className="usage-date-fields"><label>开始日期（含）<input type="date" value={startDate} onChange={e => setStartDate(e.target.value)} /></label><label>结束日期（含）<input type="date" value={endDate} onChange={e => setEndDate(e.target.value)} /></label></div>}
      <fieldset className="usage-model-filter"><legend>模型筛选 · 未勾选时统计全部模型（包含历史记录）</legend><div className="usage-model-options">{models.map(model => <label key={model.model_hash}><input type="checkbox" checked={selectedModels.includes(model.model_hash)} onChange={e => setSelectedModels(current => e.target.checked ? [...current, model.model_hash] : current.filter(hash => hash !== model.model_hash))} />{model.display_name}</label>)}{!!selectedModels.length && <button className="text-button" onClick={() => setSelectedModels([])}>全部模型</button>}</div></fieldset>
    </section>
    {error && <div className="alert error" role="alert">{error}</div>}
    <div className="metric-grid">{[
      ["总 Token", m?.token_usage, "输入 + 缓存 + 输出"], ["请求次数", m?.llm_calls, `${m?.successful_calls ?? 0} 成功 · ${m?.failed_calls ?? 0} 未成功`], ["输入 Token", m?.input_tokens, "不含缓存部分"], ["输出 Token", m?.output_tokens, "模型生成用量"],
      ["缓存读取 Token", m?.cache_read_tokens, "复用已有输入缓存"], ["缓存读取率", cacheReadRate, "缓存读取 ÷ 全部输入 Token"],
    ].map(([label, value, note]) => <div className="metric card" key={String(label)}><span>{label}</span><strong title={typeof value === "number" ? value.toLocaleString() : ""}>{typeof value === "number" ? number(value) : value ?? "—"}</strong><small>{note}</small></div>)}</div>
    <section className="card chart-card"><div className="section-heading"><div><h2>Token 使用趋势</h2><p className="hint">悬停或聚焦柱形查看明细</p></div><span className="badge">{overview?.token_usage_granularity === "day" ? "按本地日期" : overview?.token_usage_granularity === "minute" ? "按分钟" : "按小时"}</span></div>
      <div className="legend">{series.map(([, label, color]) => <span key={label}><i style={{ background: color }} />{label}</span>)}</div>
      <div className="chart" aria-label="Token 用量柱形图">{data.map((b, index) => {
        const date = new Date(b.bucket_start_ms);
        const caption = overview?.token_usage_granularity === "day" ? `${date.getMonth() + 1}/${date.getDate()}` : `${date.getHours()}:${String(date.getMinutes()).padStart(2, "0")}`;
        const detail = `${date.toLocaleString()}\n${series.map(([key, label]) => `${label}：${b[key].toLocaleString()}`).join("\n")}`;
        return <div className="chart-column" key={b.bucket_start_ms} tabIndex={0} aria-label={detail} title={detail}><div className="bar-track"><div className="bar-stack" style={{ height: `${total(b) / max * 100}%` }}>{series.map(([key, , color]) => b[key] > 0 && <i key={key} style={{ background: color, flexGrow: b[key] }} />)}</div></div><span>{index % Math.max(1, Math.ceil(data.length / 12)) === 0 ? caption : ""}</span></div>;
      })}</div>
      {!loading && data.every(b => total(b) === 0) && <p className="chart-empty">这个时间范围暂无 Token 用量，开始使用后将在这里显示。</p>}
      <div className="cache-summary"><span>缓存写入 <strong>{m ? number(m.cache_write_tokens) : "—"}</strong></span><span>读取率按当前筛选汇总计算，全部输入包含缓存读取与写入，不含输出。</span></div>
    </section>
    <section className="card usage-calendar-card"><div className="section-heading"><div><h2>用量贡献日历</h2><p className="hint">由上至下为周日到周六 · 按本地日期 · 颜色表示 Token 总量</p></div></div>
      <div className="usage-calendar-scroll"><div className="usage-calendar" aria-label="每日 Token 使用量">{Array.from({ length: overview?.calendar[0] ? new Date(`${overview.calendar[0].date}T12:00:00`).getDay() : 0 }, (_, index) => <span key={`pad-${index}`} aria-hidden="true" />)}{overview?.calendar.map(day => {
        const count = total(day); const level = count === 0 ? 0 : Math.min(4, Math.max(1, Math.ceil(count / calendarMax * 4)));
        const caption = `${day.date} · ${day.calls} 次请求 · ${count.toLocaleString()} Token`;
        return <div key={day.date} className="usage-calendar-day" tabIndex={0} title={caption} aria-label={caption} style={{ background: calendarColors[level] }} />;
      })}</div></div>
      <div className="usage-calendar-key"><span>少</span>{calendarColors.map(color => <i key={color} style={{ background: color }} />)}<span>多</span><span>· {overview?.calendar.length ?? 0} 天</span></div>
      {!loading && overview?.calendar.every(day => total(day) === 0) && <p className="hint">当前筛选范围暂无已记录 Token。</p>}
    </section>
    {range && <Pricing models={models} query={range} refreshKey={tick} />}
  </>;
}
