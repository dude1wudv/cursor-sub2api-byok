# 验证范围

## v0.2.4 — 2026-10-09

- 服务端 245 项测试最终通过。先运行 `cargo test -p cursor-server --lib --tests`，因本次 Task 模型说明调整而更新对应的工具目录快照；随后复跑 `prefix_stability` 及其后的全部集成测试，断言未跳过或弱化。
- 新增回归覆盖按类型禁用／选模型、自定义子代理模型、当前显式选择优先、推理参数 field 21 的 protobuf 往返、续接保留模型、失败结果子对话 ID，以及 Chat/Responses 的真实 loopback HTTP 请求会话标识。同对话跨 run/call 稳定、不同对话隔离、空 GPT 对话 ID 拒绝、非 GPT 请求不变。
- pnpm typecheck、typecheck:node、Vite 与 Windows Tauri no-bundle 构建通过；便携包包含 EXE、LICENSE、第三方声明和 SHA256SUMS。已更新本机至 0.2.4、备份旧 EXE、校验安装文件与包内 EXE 的 SHA-256 一致；沿用桌面快捷方式。
- 本次未重跑实际 EXE 的 CA 安装／删除与崩溃恢复套件；未修改 CA、journal 和退出流程。现有隔离恢复历史结果不能替代新版本的完整原生恢复验收。
- 本轮未调用真实上游模型，尚未验收真实 Cursor 子代理派遣／子对话打开、官方子模型权限、GPT 连续请求绑定及 GPT/Claude/MCP 闭环。静态对照本机 Cursor 3.23.12 协议，不承诺所有客户端版本兼容；上游账号不健康时仍允许正常故障转移。

## v0.2.3 — 2026-10-09

- pnpm typecheck、typecheck:node、Vite 与 Windows Tauri no-bundle 构建通过；安装 EXE 与便携包 SHA-256 一致，桌面快捷方式沿用。
- 合成浏览器回归通过：6 张用量卡片、缓存读取 850K、读取率 52.8%、无输入显示“—”、筛选模型后无缓存显示 0.0%；既有授权、模型编辑与批量发现流程通过。README 统计截图为合成数据。
- 已安装 EXE 的实际窗口 × 验证：窗口从可见列表消失，进程和本地 HTTP 服务仍运行，settings 接管与 journal 保留；原证书同意记录不变。托盘重开和显式退出需人工操作复核。
- 本次未重跑完整 CA 安装/删除和崩溃恢复套件；未发送真实上游模型请求，真实 Cursor/GPT/Claude/MCP 闭环不在本次验收结果内。

## v0.2.2 — 2026-10-09

- Rust 238 项用例最终通过：首次运行 237 项通过，控制器恢复用例因本机 Cursor 仍运行而被安全保护拦截；退出 Cursor 后该目标的 3 项用例全部通过。未跳过或弱化断言。
- 新增 4 项 Router/HTTP Body 回归，验证静默心跳、无历史污染、重叠订阅、断线宽限及旧计时失效、结束请求与未知路由的有界等待；既有取消、工具/MCP、checkpoint 和 compaction 回归通过。
- pnpm typecheck、typecheck:node、Vite 与 Windows Tauri no-bundle 构建通过。
- 已安装 EXE 本机两次开关复验通过：复用原授权和受信任 CA，无新证书操作；settings 逐字节恢复，journal 清理，模型仍为 11 个。此次未重跑全套崩溃/损坏 journal 原生测试；v0.2.1 的该项历史结果保留。
- 与流式缺陷分别定位：CommandCode 对应时段 25 条上游 400 均为 insufficient credits。CommandCode Proxy 独立提交 `72d35ac1f7` 已部署，返回明确额度提示并临时停调账号 20 分钟。没有据此改写模型推理内容或重放失败工具请求。
- 仍观察到 Gmail MCP 目标连接超时和 WebSocket 关闭错误；本次未修复外部 MCP 网络连通性，不将其宣称为已解决。
- 未发起真实模型调用；真实 Cursor/GPT/Claude/MCP 闭环及长时间线上重连效果仍待用户使用验证。


## v0.2.1 — 2026-10-09

- 代码检查：pnpm 9 的 typecheck、typecheck:node、Vite build 与 Tauri no-bundle 构建通过。
- 浏览器交互：首次证书确认后继续开启；已有授权且证书未信任时不再弹协议；安装取消可重试；Cursor 运行时阻止切换；模型编辑、发现、批量添加、统计及布局回归通过。
- 20 项 `local_app` 相关 Rust 测试通过，包括持久信任核验失败、证书未就绪时不得报告健康接管，以及既有恢复/冲突/DPAPI/JSONC 行为。
- 新版实际 EXE 隔离验证通过：开关、崩溃与损坏 journal、离线恢复、启动恢复、窗口退出、精确卸载、原始 settings 字节、账号数据库哨兵和合成秘密检查。测试证书最终已删除，CurrentUser Root 回到基线。
- 本机旧 CA 的重新安装不能保留信任；独立 Windows 接口复现同样结果，证书自身签名和有效期正常。按用户授权备份旧 CA 文件后重建，保留已有连接、Key、模型和统计。未更改系统证书策略或其他证书。
- 已安装 EXE 的本机复验通过：首次证书确认后自动开启，连续两次开关无重复应用协议，settings 逐字节恢复且证书持续受信任；未发送上游模型请求。
- README 图片为合成数据；真实 Cursor/GPT/Claude/MCP 模型调用闭环仍未验收。

GitHub Windows CI [37953044611](https://github.com/dude1wudv/cursor-sub2api-byok/actions/runs/37953044611) 全部通过，覆盖代码提交 `13c5f4a6220919a876699a1e6b34e753bb54e655`：147 项库测试 + 87 项集成测试，共 234 项；前端检查与原生桌面编译通过。后续仅补充验证文档。

## v0.2.0 历史验证

v0.2.0，2026-10-09，Windows x64，固定上游 `7ee68c2b7fef66a0e0279273d037d23fbc2f11ad`。

## 代码与自动化

- Rust：146 项库测试和 87 项集成测试通过，共 233 项。完整回归后修正数据库版本断言及 Windows 短路径断言，并复验库测试。无测试跳过。
- 前端：pnpm 9，typecheck、typecheck:node 与 Vite build 通过；浏览器合成接口验证通过。
- 新增覆盖：强度允许列表/默认值校验、Cursor 目录默认变体、持久化与连接同步、模型列表鉴权/去重/错误/禁止重定向/响应大小限制、批量配置失败零写入。
- 证书逻辑：首次明确同意，DPAPI 记录绑定 DER；关闭和恢复不删除持续信任，损坏同意记录保留。旧 journal 仍按原临时信任语义恢复。
- 浏览器交互：首次同意不可预选、编辑错误可见、取消默认档位后回到模型默认、模型列表失败重试/搜索/批量添加、用量筛选与布局检查。README 图片使用合成数据。

## 构建与隔离恢复

Tauri `--no-bundle` 构建通过，已生成 EXE、ZIP、原 LICENSE、第三方许可及 SHA256SUMS，并核对本机安装文件与发布包一致。

v0.2.0 实际 EXE 的隔离原生验证已通过，使用专用临时数据目录、Cursor profile 与合成 Key：

- 首次应用同意 + Windows 确认安装后，多次开启/关闭无需再次安装或删除证书。
- 关闭后 settings 的 BOM、CRLF、注释及全部原始字节恢复，专属证书继续受信任。
- 强制终止后，损坏 journal 被保留并返回非零；有效 journal 的离线 `--restore` 恢复成功且可重复执行。
- 启动恢复不自动接管；真实窗口 × 在恢复后退出，持续证书保留。
- 主动“卸载证书并恢复”后 CurrentUser Root 集合精确回到基线，同意记录移除。
- `state.vscdb` 哨兵不变；DB/WAL/日志无合成 Key 明文；CA 私钥由 DPAPI 保护。

GitHub Windows CI [37946164373](https://github.com/dude1wudv/cursor-sub2api-byok/actions/runs/37946164373) 全部通过，验证代码提交 `6ea8a4868bc92e3ae3aaed3cee21297bb45f60f3`。后续仅更新文档，不重复运行相同代码验证。五个 GitHub Release 附件的服务端 SHA256 均与本地文件相符。

运行 `scripts/verify-portable.cjs EXE EVIDENCE_DIRECTORY PLAYWRIGHT_PACKAGE` 需要完全退出 Cursor，并由用户确认首次 Windows 证书安装与最终删除。检查日常多次开关、强制终止、离线恢复、启动恢复和窗口退出期间证书始终保留，显式卸载后 Root 集合回到基线；同时检查 settings 字节与 state.vscdb 哨兵。

## 未验证范围

**真实 Cursor/GPT/Claude/MCP 闭环未验收**：没有专用 Key 和真实工作区验收输入。协议 fixture、隔离 EXE、模型目录测试不能替代真实上游模型和 Cursor 原生界面的使用闭环。

MicroEduLab 是维护者署名，EXE 未做 Authenticode 数字签名。
