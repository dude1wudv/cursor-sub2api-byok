# Cursor Sub2API BYOK

Windows x64 便携控制器，只负责 Cursor → Sub2API。由 **[MicroEduLab](https://microedulab.com/)** 开发与维护。

[下载 Release](https://github.com/dude1wudv/cursor-sub2api-byok/releases) · [开发仓库](https://github.com/dude1wudv/cursor-sub2api-byok) · [反馈问题](https://github.com/dude1wudv/cursor-sub2api-byok/issues) · [更新记录](CHANGELOG.md) · [验证范围](docs/VALIDATION.md)

基于 [leookun/cursor-byok](https://github.com/leookun/cursor-byok) 固定提交 `7ee68c2b7fef66a0e0279273d037d23fbc2f11ad`，保留上游 Git 历史、原 MIT LICENSE 和版权。本项目独立维护，不隶属于 Cursor。

## 下载与要求

从 Release 下载 `Cursor-Sub2API-BYOK-windows-x64.zip`，解压后运行 EXE。包内含原 LICENSE、THIRD-PARTY-NOTICES.txt 和 SHA256SUMS.txt；Release 另提供完整资产校验和。

- Windows x64、Microsoft Evergreen WebView2、有效的 Sub2API Base URL/Key，以及自己的 Cursor 登录环境。
- MicroEduLab 是维护者署名；**EXE 未做 Authenticode 数字签名**，可能出现 SmartScreen 提示。
- 开发预发布版本；验证结果见下方“验证范围”。**真实 Cursor/GPT/Claude/MCP 闭环尚未验收**，详见验证范围。

## 功能

- 一个连接共享给多个 GPT/Claude 模型，默认使用 Responses/Messages，可选 Chat Completions。
- `/v1/models` 获取可用模型，搜索、勾选并批量添加；仍支持手动配置。
- 每个模型独立设置可选推理强度与默认强度，在 Cursor 原生选择器切换。
- 浅色模型卡片、连接测试反馈、按时间和模型筛选的输入/输出/缓存 Token 统计。
- 启停通过可恢复事务管理 JSONC settings 和本地代理；专属证书首次授权安装，卸载时清理。
- Key、CA 私钥和 journal 由 Windows CurrentUser DPAPI 加密。
- 管理 API 仅在 loopback 上使用随机 token，并校验 Host/Origin。
- 保留 Cursor 自带 rules/Skills/MCP、工具、取消、检查点与 compaction；不添加软件 rules。

## 使用

1. 安装 Microsoft [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)；程序不会静默安装运行时。
2. 完全退出 Cursor，打开 `Cursor-Sub2API-BYOK.exe`，阅读并勾选首次使用说明，安装当前用户专属证书。
3. 保存 Sub2API HTTPS 根地址或 `/v1` 与专用 API Key，点击“获取模型列表”勾选添加。按模型名称预选 Responses/Claude Messages，添加前可调整协议。编辑模型卡片设置可选强度与默认值；只勾选上游支持的档位。
4. 完全退出 Cursor，点击“开启接管”，再手动打开 Cursor 并使用原生登录。
5. 完全退出 Cursor 后点击“关闭并恢复”，或关闭本程序窗口。窗口 × 会恢复后退出；Cursor 仍运行或恢复失败时阻止退出。

本程序不修改 Cursor 安装文件、hosts、系统代理或账号数据库，不伪造会员资格。官方账号/计费流量继续转发到 Cursor。原生工具、MCP、Skills、检查点、取消和 compaction 仍使用上游实现。

## 界面

以下预览使用合成模型与统计数据。

![模型配置卡片](docs/images/models.png)
![Token 统计](docs/images/usage.png)

## 数据与恢复

默认数据目录 `%LOCALAPPDATA%\CursorSub2APIByok`；默认目标 `%APPDATA%\Cursor\User\settings.json`。API Key、CA 私钥及恢复 journal 使用 Windows CurrentUser DPAPI，仅当前 Windows 用户可解密，不能直接迁移到其他用户或机器。

```powershell
.\Cursor-Sub2API-BYOK.exe --data-dir "E:\Isolated Test\Data" --cursor-user-data-dir "E:\Isolated Test\Cursor Profile"
.\Cursor-Sub2API-BYOK.exe --restore --data-dir "E:\Isolated Test\Data" --cursor-user-data-dir "E:\Isolated Test\Cursor Profile"
```

`--cursor-user-data-dir` 指向用户数据根，实际操作其 `User\settings.json`。路径须为绝对路径且不能经过 reparse point；GUI 与恢复命令全局单实例互斥。

首次同意使用说明后生成专属 CA，只安装到 CurrentUser Root。Windows 首次安装和最终删除时可能要求确认；核对名称为 `Cursor Sub2API BYOK Local CA` 后点击“是”。日常关闭、窗口退出、崩溃恢复与 `--restore` 恢复 settings 并停止代理，**保留已同意持续安装的证书**，因此无需反复确认。v0.1.0 遗留事务仍按当时记录恢复临时信任。卸载前请进入“关于 → 卸载证书并恢复”，成功后再删除便携程序；模型配置和统计保留。证书按 DPAPI 同意记录的完整 DER 精确删除，失败保留记录供重试；不要提前删除数据目录。原 JSONC 的 BOM、CRLF、注释及无关字段保留。用户在接管期间改成第三值的管理项保留并提示，无法确认归属时保留 journal 并进入 `recovery_required`。

崩溃后使用相同参数重新启动或执行 `--restore`。恢复命令不启动代理或网络客户端，成功返回 0，冲突返回非零。不要删除 `takeover-journal.dpapi` 或复制其他用户的数据目录来绕过恢复。

## 本地构建

需要 Windows x64、MSVC/Rust、pnpm 9、Node 和 Python 3。

```powershell
cargo test -p cursor-server --lib --tests
pwsh -NoProfile -File scripts/build-portable.ps1
```

脚本执行 frozen install、两个 TypeScript 检查及 `tauri build --no-bundle`，交付 EXE、LICENSE、THIRD-PARTY-NOTICES.txt、SHA256SUMS.txt 和相邻 ZIP。默认输出 `dist/windows-x64`，可用 `-OutputDirectory <absolute-path>` 指定目录。没有安装器、自动更新或开机自启。

自动化测试使用临时数据、合成 Key 与 loopback fixture。真实 Cursor/GPT/Claude/MCP 闭环必须使用用户自行登录的专用环境和专用 Key 单独验收。

## 开发结构

```text
apps/desktop/src/          # 连接、模型、接管状态及 MicroEduLab 关于页
apps/desktop/src-tauri/    # Windows 入口、单实例、窗口与退出流程
server/src/local_app/     # JSONC、DPAPI、CA、journal 与代理事务
server/src/control/       # 管理鉴权与控制 API
server/src/store/         # SQLite 与共享连接/模型原子同步
server/src/cursor/        # Cursor 原生协议、工具与会话生命周期
scripts/                 # 便携打包、依赖许可及隔离原生验证
```

数据流：Cursor → 本地 loopback 代理 → 原生协议适配 → Sub2API；账号、计费等官方流量继续转发 Cursor。GitHub Windows CI 执行前端检查、隔离服务端测试及原生编译检查；公开 Release 由维护者验证本地资产后发布，不使用上游 updater 发布链。

## 反馈与许可

反馈请提供版本、Windows 版本、操作步骤及脱敏错误。**不要上传 Key、CA 私钥、journal、数据库、Cursor 账号数据或未脱敏日志。**

MIT，原版权 `Copyright (c) 2026 leookun` 保持不变。MicroEduLab 维护本 fork；第三方依赖说明随发行包提供。[MicroEduLab 网站](https://microedulab.com/)
