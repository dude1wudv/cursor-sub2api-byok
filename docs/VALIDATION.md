# 验证范围

2026-10-09，Windows x64，固定上游 `7ee68c2b7fef66a0e0279273d037d23fbc2f11ad`。

| 层次 | 结果 |
| --- | --- |
| Rust 自动化 | `cargo test -j 2 -p cursor-server --lib --tests --no-fail-fast`：228 passed，0 failed |
| 前端 | pnpm 9 frozen install、typecheck、typecheck:node、Vite build 通过 |
| 原生构建 | Tauri `--no-bundle`，PE AMD64，EXE/ZIP 与 SHA256 校验通过 |
| 隔离原生恢复 | 实际 EXE、带空格的临时路径、专用 Cursor profile、合成 Key；用户确认 Windows 专属 CA 提示后通过 |
| 真实 Cursor/GPT/Claude/MCP | **未验收**：没有专用 Key 与登录后的真实工作区；fixture 不能替代真实闭环 |

隔离验证包括逐字节还原带 BOM/CRLF/注释的 settings、CurrentUser Root 证书集合恢复、强制终止后的 journal 恢复、损坏 journal 保留且返回非零、启动只恢复不自动接管、窗口 × 恢复后退出。合成 `state.vscdb` 未变；DB/WAL/日志未发现合成 Key 明文。

直接测试覆盖 JSONC 冲突、用户第三值、各 journal 阶段、只读/竞争、重复启停、Cursor 运行保护、精确 DER、同名不同证书、SQLite 原子回滚、控制鉴权，以及 GPT Responses/Claude Messages 共用 Key、官方 Free/Pro/401/503 原样透传。原生工具/MCP、取消、检查点和 compaction 回归通过。

原生交互测试：`scripts/verify-portable.cjs EXE EVIDENCE_DIRECTORY PLAYWRIGHT_PACKAGE`。仅在独立测试目录且 Cursor 完全退出时运行，需要用户确认 Windows 证书安装和删除提示；不得无人值守运行后遗留 journal。

MicroEduLab 署名版仅更新关于页、固定外链、文档和打包入口。发布前重做前端检查与原生构建；复用上述同一恢复实现的隔离证据，不将其描述为真实模型验收。
