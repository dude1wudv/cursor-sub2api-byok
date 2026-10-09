# 验证范围

v0.2.0，2026-10-09，Windows x64，固定上游 `7ee68c2b7fef66a0e0279273d037d23fbc2f11ad`。

## 代码与自动化

- Rust：146 项库测试和 87 项集成测试通过，共 233 项。完整回归后修正数据库版本断言及 Windows 短路径断言，并复验库测试。无测试跳过。
- 前端：pnpm 9，typecheck、typecheck:node 与 Vite build 通过；浏览器合成接口验证通过。
- 新增覆盖：强度允许列表/默认值校验、Cursor 目录默认变体、持久化与连接同步、模型列表鉴权/去重/错误/禁止重定向/响应大小限制、批量配置失败零写入。
- 证书逻辑：首次明确同意，DPAPI 记录绑定 DER；关闭和恢复不删除持续信任，损坏同意记录保留。旧 journal 仍按原临时信任语义恢复。
- 浏览器交互：首次同意不可预选、编辑错误可见、取消默认档位后回到模型默认、模型列表失败重试/搜索/批量添加、用量筛选与布局检查。README 图片使用合成数据。

## 构建与隔离恢复

Tauri no-bundle 构建及实际 EXE 验证结果在本次发布前补充。隔离验证使用专用临时数据目录和 Cursor profile；不使用真实 Key 或真实 Cursor 会话。

运行 `scripts/verify-portable.cjs EXE EVIDENCE_DIRECTORY PLAYWRIGHT_PACKAGE` 需要完全退出 Cursor，并由用户确认首次 Windows 证书安装与最终删除。检查日常多次开关、强制终止、离线恢复、启动恢复和窗口退出期间证书始终保留，显式卸载后 Root 集合回到基线；同时检查 settings 字节与 state.vscdb 哨兵。

## 未验证范围

**真实 Cursor/GPT/Claude/MCP 闭环未验收**：没有专用 Key 和真实工作区验收输入。协议 fixture、隔离 EXE、模型目录测试不能替代真实上游模型和 Cursor 原生界面的使用闭环。

MicroEduLab 是维护者署名，EXE 未做 Authenticode 数字签名。
