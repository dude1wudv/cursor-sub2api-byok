# Changelog

## 0.1.0 — 2026-10-09

首次公开开发版本，由 [MicroEduLab](https://microedulab.com/) 维护。

- Windows x64 便携控制器，一个 Sub2API 连接共享给多个 GPT/Claude 模型。
- JSONC 保留式接管、DPAPI journal、冲突检测、启动恢复和离线 `--restore`。
- CurrentUser Root 专属 CA，按精确 DER 删除，保留预先存在的信任。
- CurrentUser DPAPI 加密 Key、CA 私钥及整个恢复 journal。
- 模型配置与共享连接原子同步；管理 API 使用进程 token、Host/Origin 校验。
- 完全退出 Cursor 后切换；关闭控制器窗口会恢复后退出，冲突阻止退出。
- 保留官方账号流量及 Cursor 原生工具/MCP/Skills 协议；移除账号注入、会员伪造、广告、插件市场、自动更新、外部 BYOK 和软件 rules。

验证：228 项本地自动化测试通过；Windows 实际 EXE 的隔离启停、证书恢复、崩溃恢复和窗口退出恢复通过。真实 Cursor/GPT/Claude/MCP 闭环尚未验收。EXE 未做 Authenticode 数字签名，MicroEduLab 为维护者署名。
