# 验证范围

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
