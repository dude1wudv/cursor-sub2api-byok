# Cursor Sub2API BYOK v0.1.0 · MicroEduLab

由 **[MicroEduLab](https://microedulab.com/)** 开发与维护的首个公开开发版本。Windows x64 便携程序，让 Cursor 使用一个共享 Sub2API 连接和多个 GPT/Claude 模型，并支持可逆接管。

## 下载

推荐下载 **Cursor-Sub2API-BYOK-windows-x64.zip**，解压后运行 EXE。ZIP 包含 EXE、原 MIT LICENSE、第三方依赖许可和内部 SHA256 校验文件。Release 另附独立 EXE、许可文件和覆盖 ZIP 的 SHA256SUMS.txt。

要求 Windows x64、[Microsoft Evergreen WebView2](https://developer.microsoft.com/microsoft-edge/webview2/)、用户自己的 Sub2API Key 和 Cursor 登录环境。EXE **未做 Authenticode 数字签名**，可能出现 SmartScreen 提示；MicroEduLab 为维护者署名。

## 本版功能

- 一个 Sub2API 连接共享给多个模型，GPT 默认 Responses、Claude 默认 Messages，可选 Chat Completions。
- Windows CurrentUser DPAPI 保护 Key、CA 私钥和整个恢复 journal。
- 保留 JSONC 注释、BOM、CRLF 和无关设置，支持冲突检测、崩溃恢复及离线 `--restore`。
- 专属 CA 仅安装到 CurrentUser Root；恢复时按精确 DER 清理本次新增证书。
- 完全退出 Cursor 后切换接管；关闭控制器窗口会恢复后退出，Cursor 运行中或恢复失败会阻止退出。
- 保留官方账号流量和原生工具/MCP/Skills；移除账号注入、会员伪造、广告、插件市场、自动更新、外部 BYOK 及软件 rules。

## 使用提示

先保存连接并添加模型，完全退出 Cursor 后开启接管，再打开 Cursor。关闭接管前也需完全退出 Cursor。Windows 会确认安装或删除证书；核对名称 **Cursor Sub2API BYOK Local CA** 后选择“是”。不要删除恢复 journal，也不要把 DPAPI 数据目录复制到其他用户或机器。

## 验证与限制

- 228 项本地 Rust 测试通过；前端类型检查与 Tauri no-bundle 构建通过。
- 实际 EXE 的隔离启停、CurrentUser CA/原 settings 恢复、强制终止后 `--restore`、启动恢复及窗口退出恢复通过。署名版已重新构建并验证关于页与隔离启动。
- **真实 Cursor/GPT/Claude/MCP 闭环尚未验收**：目前没有专用 Key 和登录后的真实工作区。测试 fixture 不代表真实上游兼容性保证，因此本版标记为预发布。
- 没有安装器、后台 watchdog、自动更新或开机自启。

来源：[leookun/cursor-byok](https://github.com/leookun/cursor-byok)，固定上游提交 `7ee68c2b7fef66a0e0279273d037d23fbc2f11ad`；原 MIT LICENSE 与 `Copyright (c) 2026 leookun` 保持不变。

[完整说明](https://github.com/dude1wudv/cursor-sub2api-byok#readme) · [问题反馈](https://github.com/dude1wudv/cursor-sub2api-byok/issues) · [MicroEduLab](https://microedulab.com/)
