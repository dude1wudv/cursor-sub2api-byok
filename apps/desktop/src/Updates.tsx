import { useCallback, useEffect, useRef, useState } from "react";
import { request } from "./api";
import "./updates.css";

type Preferences = { automatic: boolean; include_prereleases: boolean };
type Status = {
  current_version: string; preferences: Preferences; checked_at_ms: number | null;
  error: string | null; update_available: boolean;
  latest: { version: string; prerelease: boolean; release_url: string; download_url: string } | null;
};
const interval = 6 * 60 * 60 * 1000;
export function useUpdates() {
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const pending = useRef(false);
  const current = useRef(status); current.current = status;
  const check = useCallback(async () => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError("");
    try { setStatus(await request<Status>("updates", "POST")); }
    catch (e) { setError(e instanceof Error ? e.message : "更新检查失败"); }
    finally { pending.current = false; setBusy(false); }
  }, []);
  useEffect(() => {
    let active = true;
    void request<Status>("updates").then(value => {
      if (!active) return;
      setStatus(value);
      if (value.preferences.automatic) void check();
    }).catch(e => { if (active) setError(String(e.message)); });
    const timer = window.setInterval(() => { if (current.current?.preferences.automatic) void check(); }, interval);
    return () => { active = false; window.clearInterval(timer); };
  }, [check]);
  const preferences = async (value: Preferences) => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError("");
    try { setStatus(await request<Status>("updates", "PUT", value)); }
    catch (e) { setError(e instanceof Error ? e.message : "更新设置保存失败"); }
    finally { pending.current = false; setBusy(false); }
  };
  return { status, busy, error, check, preferences };
}
export function Updates({ updates }: { updates: ReturnType<typeof useUpdates> }) {
  const { status, busy, error, check, preferences } = updates;
  return <section className="card updates-card">
    <div className="section-heading"><div><h2>软件更新</h2><p className="hint">仅检查 dude1wudv/cursor-sub2api-byok 的完整 Windows 发布包</p></div><button disabled={busy} onClick={() => void check()}>{busy ? "正在检查…" : "检查更新"}</button></div>
    <div className="update-options">
      <label><input type="checkbox" checked={status?.preferences.automatic ?? true} disabled={!status || busy} onChange={e => status && void preferences({ ...status.preferences, automatic: e.target.checked })} />启动时及运行期间每 6 小时自动检查</label>
      <label><input type="checkbox" checked={status?.preferences.include_prereleases ?? true} disabled={!status || busy} onChange={e => status && void preferences({ ...status.preferences, include_prereleases: e.target.checked })} />包含预发布版本</label>
    </div>
    <p className="hint">当前版本 {status?.current_version ?? "—"} · 上次检查 {status?.checked_at_ms ? new Date(status.checked_at_ms).toLocaleString() : "尚未检查"}</p>
    {(error || status?.error) && <div className="alert error" role="alert">{error || status?.error}。当前更新状态未确认，可稍后重试。</div>}
    {status?.latest && <div className="update-result" role="status"><strong>{error || status.error ? `上次已知版本 v${status.latest.version}` : status.update_available ? `发现新版本 v${status.latest.version}` : `当前无更高版本（所选渠道最新 v${status.latest.version}）`}</strong><span className="badge">{status.latest.prerelease ? "预发布" : "正式版"}</span><a href={status.latest.release_url} rel="noreferrer">版本说明 ↗</a>{status.update_available && <a href={status.latest.download_url} rel="noreferrer">下载 Windows ZIP ↗</a>}</div>}
    {status?.checked_at_ms && !error && !status.error && !status.latest && <p role="status">所选渠道暂无包含完整 Windows 资产的版本。</p>}
    <p className="hint">通过应用出站代理读取公开 Release 信息，不发送 Key、账号或对话。检查失败不改为直连。只提示和提供下载入口，不自动安装；替换前请完全退出 Cursor，再从托盘“退出并恢复”。下载后按发布页 SHA256SUMS 校验。</p>
  </section>;
}
