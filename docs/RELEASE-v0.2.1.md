# Cursor Sub2API BYOK v0.2.1

由 [MicroEduLab](https://microedulab.com/) 维护的 Windows x64 便携开发版本。

- 修复“开启接管”反复弹出应用协议：首次确认安装后继续开启，已有有效授权直接使用，不重复同意。
- 安装证书后重新读取 Windows 信任存储核验；未受信任时不报告健康接管。Cursor 仍运行时显示退出提示。
- 全新浅色蓝紫玻璃工作台：连接摘要可展开，模型信息分区，多档推理强度与默认值更清晰。
- 优化 Token 卡片、四色趋势图及页面滚动；保留无广告界面。

升级前完全退出 Cursor 与旧控制器，下载 ZIP 解压替换，或使用独立 EXE。已有 Key、模型与统计保留。专属 CA 首次系统安装仍可能需要 Windows 确认，日常关闭和退出继续保留证书，主动卸载时才删除。

20 项相关 Rust 测试、前端检查、浏览器回归、no-bundle 构建、新版 EXE 隔离恢复和本机两次开关验证均通过。验证未发送真实上游模型请求。

验证记录见 [验证范围](https://github.com/dude1wudv/cursor-sub2api-byok/blob/main/docs/VALIDATION.md)。**真实 Cursor/GPT/Claude/MCP 闭环仍未验收。**

附件：EXE、ZIP、原 MIT LICENSE、THIRD-PARTY-NOTICES.txt、SHA256SUMS.txt。MicroEduLab 是维护者署名，EXE 未做 Authenticode 数字签名。
