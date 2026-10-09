# Cursor Sub2API BYOK v0.2.0

由 [MicroEduLab](https://microedulab.com/) 维护的 Windows x64 便携开发版本。

- 推理强度支持多选允许档位，并单独设置默认强度，在 Cursor 原生模型选择器切换。
- 一键请求 `/v1/models`，搜索、勾选和批量添加模型，支持调整 GPT/Claude 协议。
- 新浅色工作台：独立模型卡片、连接测试反馈、输入/输出/缓存 Token 趋势和模型筛选，无广告。
- 证书改为首次同意说明后安装，日常开关与退出保留。卸载前在“关于”点击“卸载证书并恢复”；Windows 首次安装和最终删除仍可能要求确认。
- 保留已有连接、模型和统计。旧版接管事务先恢复，新版本不自动开启接管。

下载 ZIP 解压运行，或替换已完全退出的旧 EXE。数据仍在 `%LOCALAPPDATA%\CursorSub2APIByok`；升级前请完全退出 Cursor 和旧控制器。

233 项 Rust 测试、前端类型检查、浏览器交互和 no-bundle 构建通过。新版持续证书信任的完整原生隔离流程因 Windows 确认未完成而尚未验收；失败退出后证书集合已核对回到基线。

构建、自动化与隔离验证结果见 [验证范围](https://github.com/dude1wudv/cursor-sub2api-byok/blob/main/docs/VALIDATION.md)。**真实 Cursor/GPT/Claude/MCP 闭环未验收**，合成测试不代表真实上游可用性。

附件：EXE、ZIP、原 MIT LICENSE、THIRD-PARTY-NOTICES.txt、SHA256SUMS.txt。EXE 未做 Authenticode 数字签名，MicroEduLab 是维护者署名。
