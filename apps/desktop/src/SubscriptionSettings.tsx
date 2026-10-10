import { useState } from "react";
import { request, type Status } from "./api";

export function SubscriptionSettings({ status, refresh }: { status: Status | null; refresh: () => Promise<void> }) {
  const [consent, setConsent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const pending = status?.subscription_recovery_pending ?? false;
  async function change() {
    setBusy(true); setError("");
    try {
      await request("harness/cursor/subscription", "PUT", { enabled: !pending, consent });
      setConsent(false);
      await refresh();
    } catch (e) { setError(e instanceof Error ? e.message : "订阅缓存操作失败"); await refresh(); }
    finally { setBusy(false); }
  }
  return <section className="card"><h2>临时订阅缓存</h2>
    <span className="badge">{pending ? "等待恢复 · 官方刷新可能已更改缓存" : "默认关闭 · 每次手动启用"}</span>
    <p>将当前 Cursor profile 的本地订阅缓存临时设为 Ultra / active，保留启动前的登录凭据及账号归属。这会影响客户端功能开关；官方服务仍按真实账号权限和额度处理。</p>
    <p className="hint">官方刷新可能覆盖缓存，本工具不会持续强写或修改官方响应。关闭注入、关闭接管或托盘“退出并恢复”时逐字段恢复；保留官方及第三方改动。Cursor 运行时恢复会暂停，完全退出 Cursor 后可重试。窗口 × 仅隐藏到托盘。</p>
    <p className="hint">目标 profile：{status?.settings_path || "正在读取"}。不切换账号，不读取或保存 accessToken / refreshToken。</p>
    {!pending && <label><input type="checkbox" checked={consent} disabled={busy} onChange={e => setConsent(e.target.checked)} />我了解这只是本地缓存，选择在本次运行期间启用，并同意保存加密恢复记录。</label>}
    {error && <div className="alert error">{error}</div>}
    <div className="form-footer"><span>{status?.restart_required ? "请先保存工作并完全退出 Cursor" : "原值使用 CurrentUser DPAPI 保护"}</span><button disabled={busy || !status || (!pending && !consent)} onClick={() => void change()}>{busy ? "正在处理…" : pending ? "关闭注入并恢复" : "启用临时缓存"}</button></div>
  </section>;
}
