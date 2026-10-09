import { useState } from "react";
import { request } from "./api";

export function CertificateSetup({ done, cancel }: { done: () => Promise<void>; cancel: () => void }) {
  const [accepted, setAccepted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return <div className="overlay"><div className="dialog" role="dialog" aria-modal="true" aria-labelledby="certificate-title">
    <span className="eyebrow">ONE-TIME SETUP · V1</span><h2 id="certificate-title">使用说明与证书授权</h2>
    <div className="consent-copy"><p>Cursor Sub2API BYOK 通过本机代理，将你选择的 Cursor 模型请求转发到你配置的 Sub2API 服务。</p>
      <ul><li>安装专属的 <strong>Cursor Sub2API BYOK Local CA</strong> 到当前 Windows 用户的受信任根证书存储。Windows 首次安装时可能要求你点击确认。</li>
        <li>证书私钥与 API Key 使用当前 Windows 用户的 DPAPI 加密。根证书拥有本用户范围的信任能力，请妥善保管本机与数据目录。</li>
        <li><strong>日常关闭和退出会恢复 Cursor 设置、停止代理，保留证书。</strong>后续开关不再重复安装或删除证书。</li>
        <li>不再使用时，在“关于”中选择“卸载证书并恢复”，按完整证书精确清理。Windows 最终删除时仍可能要求确认。请先清理再删除程序或数据目录。</li>
        <li>你的请求由所配置的 Sub2API 处理；本机统计仅记录本程序的调用。软件按 MIT 许可证提供，不保证模型服务可用性。</li></ul>
      <p className="hint">开始安装前请保存工作并完全退出 Cursor。本说明不会替代 Windows 的系统确认，也不会自动勾选同意。</p></div>
    {error && <div className="alert error" role="alert">{error}</div>}
    <label className="consent-check"><input type="checkbox" checked={accepted} disabled={busy} onChange={e => setAccepted(e.target.checked)} />我已阅读并同意以上说明，允许持续保留专属证书，直到我主动卸载。</label>
    <div className="dialog-footer"><button disabled={busy} onClick={cancel}>稍后</button><button className="primary" disabled={busy || !accepted} onClick={async () => {
      setBusy(true); setError("");
      try { await request("harness/cursor/ca/consent", "POST", { accepted: true, version: 1 }); await done(); }
      catch (e) { setError(e instanceof Error ? e.message : "证书安装失败"); }
      finally { setBusy(false); }
    }}>{busy ? "等待 Windows 确认…" : "同意并安装证书"}</button></div>
  </div></div>;
}
