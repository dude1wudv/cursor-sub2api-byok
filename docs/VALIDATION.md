# 验证范围

## v0.3.0-rc.1 — 2026-10-10（预发布）

### 代码与构建

- `cargo test --workspace --all-targets`：330 passed，0 failed，0 ignored；包括服务端、桌面和 Semble 工作区目标。包含本轮消息 ID、别名路由及原生参数回归。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。补装当前工具链 clippy 后修复新代码及现有测试中的 lint，不关闭规则。
- 前端两个 TypeScript 检查和 Vite 构建通过；Windows Tauri `--no-bundle` release 构建通过。MSVC 链接器输出与 Cargo PDB 文件名提示不影响成功退出，EXE 未做 Authenticode 签名。
- 合成浏览器交互通过：调用筛选/分页/父子诊断，模型复制/分组排序/批量取消，多模型用量/日历/价格，代理/端口，以及订阅默认关闭、明确同意和错误展示。

### 子代理根因及直接回归

Cursor 3.23.12 可独立发送 `x-parent-request-id` 与 `x-parent-agent-tool-call-id`。旧版本要求二者同时存在，真实日志四次出现本地 `protocol error: Cursor subagent request must include both parent headers`（UTC 02:16:09、21、36、54），约十秒后相同子请求的 RunSSE 返回 RunNotFound 404。故此前模型所述 404 不能作为缺少 REST 接口的证据。

修复覆盖本地 `POST /aiserver.v1.BidiService/BidiAppend` 的部分关联信息接收、冲突检测、迟到信息合并，以及 `POST /agent.v1.AgentService/RunSSE` 对已知拒绝原因的及时结束。新增元数据诊断包含请求/父请求/Task ID、路径、来源、阶段、状态、耗时，不采集完整内容。回归还覆盖初始空字段不能创建悬挂 actor、完成及取消后的同 attempt ID 重放不能再次执行、checkpoint 使用迟到 Task ID、活跃流较高 sequence 的新 action 仍可执行。已执行的 transport ID 通过 runs 持久化记录拒绝重放；Cursor 重试/续接使用新的 attempt/generation，保留原 conversation。

### 隔离恢复

- 订阅缓存 10 项合成 SQLite 集成测试、1 项 Prepared/Restoring 崩溃幂等测试通过；另有 Harness 生命周期及 Cursor 运行保护测试。
- 实际新 EXE + 临时 profile + 合成 SQLite 验证通过：明确同意才写两字段，DPAPI journal，派生 JSON 叶恢复保留其他字段与 null，第三方订阅修改保留，强制结束测试专属进程后 `--restore` 幂等，启动自动恢复。原 settings 与合成 accessToken 不变，无真实账号操作或模型用量。
- 本轮未重跑 Windows 首次 CA 安装/卸载全流程；沿用既有授权证书的接管成功。旧版本完整 CA 验收仅作为历史证据。

### 真实 Cursor 验收

已用完授权范围：GLM z-ai/glm-5.3-flash，6次人工父请求、8次 Task 派遣，仅临时合成工作区，包含首轮失败的2次派遣。后台结果自动唤起父对话及 HTTP 自动重试单独计数，不冒充新增人工请求或 Task。GPT/Claude/MCP、活跃流断线重连和官方子模型权限的真实闭环未验证。

首轮实际使用1次父请求、2次 Task 派遣，均失败，父模型未代读文件。第一子请求把 provider model 名称直接发送到 Cursor 官方路由，BidiAppend/RunSSE HTTP 200 内返回 Model name is not valid；第二次使用正确 BYOK hash 后，4次自动传输尝试在本地 prepare 阶段报 UserMessage 缺少 message_id。这两项均不等同于 HTTP 404。Cursor 3.23.12 agent-host 的 createConversationAction 原生省略该字段，其 runId 跨传输重试保持不变。

针对实证增加唯一 BYOK 名称到路由 ID 的解析（歧义拒绝，保留明确原生选项）、基于子 conversationId/runId 的稳定 input ID，以及省略 message_id 的实际 HTTP 初始化直到完成回归。下列真实结果来自重建后的 EXE，不能用隔离通过代替尚未通过的项目。

后续审查修正别名边界：原生类型默认及显式使用同一官方 ID 均保留原路由；自定义文件名称与工具模型名称才参与别名解析；解析为非父 BYOK 模型时查回该模型原生 parameters，保留 effort/context 等字段。禁用类型仍优先拒绝。

重建后的真实 GLM 验收：新建 explore 成功，只读 alpha.txt 返回 blue/purple/cyan，子对话显示读取和完整结果，父卡 Completed 且回传正确。随后 Task resume 沿用同一子 conversation，读取 beta.txt 并保留颜色历史；两次子 run 均 completed。两个后台子代理在相隔50ms内启动，父先返回，之后两次后台完成自动唤起父对话并汇总正确，无重复派遣。这些是实际 Cursor 3.23.12 + Sub2API 的运行证据。

单独取消仍未通过：2026-10-09 19:28:54（UTC−8）点击原生子卡片 Stop 后短暂显示 Stopped，但子任务继续完成。Cursor `renderer.log` 同时明确记录 `[AgentHostService] dropping conversation action with no host delivery path`，`actionCase=cancelSubagentAction`。该动作在客户端被丢弃，不能推断为服务端已收到取消；后端日志及 DB 显示 child completed，与界面最终结果一致。本次有两次 Sub2API 502 自动请求重试，没有新增 Task 派遣。

最后一次采用合成自定义类型 `acceptance-reader`，文件默认 `model: z-ai/glm-5.3-flash`，Task 未显式指定模型。子请求 `a012431f-ee5b-4b13-9b07-f065c0324eeb` 实际进入 BYOK hash `76ff545d80da710a`，BidiAppend/RunSSE 均为 local_byok HTTP 200；原生子对话可打开，展示 README 读取、10条说明和 GLM 模型名。此轮从原生输入区后台列表点击 Stop（2026-10-10 03:36:05 UTC），子调用仍持续至 03:36:32.793 UTC，随后触发父结果回传；该入口取消也未通过，其具体丢失环节尚未确证。模型文字所称“停止前已完成”与时间证据不符，不采信为成功取消。

最终元数据核对：重建后的14个 run 全部 completed（含后台自动回传），running run/call 均为0，无本轮遗留等待。此结果证明任务最终收尾，不证明取消有效。原生单独取消目前是明确未解决项，本轮不能宣称子代理全部闭环完成。

构建 EXE SHA-256：`a7c16ed7034412fd0c4a744ccaf353d1e2a0fb3138d4cc3b91c19175890d174d`。本地元数据证据保存在忽略目录 `tmp/validation-0.3.0/glm-real-metadata.json`，仅含请求关联、模型、路由、状态与时间；没有导出完整对话、Key 或账号 token。

### Cursor 最新版与本机安装

发布前重新核对：本机已升级为 Cursor 3.24.12，官方[下载页](https://cursor.com/download)将 3.24 列为 Latest。对当前安装代码的只读审查确认：初始子消息仍省略 messageId，SubagentArgs 的 model_parameters 仍是 field 21；已有兼容逻辑覆盖这两项协议。3.24.12 的 AgentHostService 仍未转发 cancelSubagentAction（workbench.desktop.main.js 字符位置19455360附近），因此没有将上一版本取消失败宣称为新版已修复。未修改 Cursor 安装文件；3.24.12 的真实模型行为尚未复验。

用户已接受带上述限制公开发布预发布版。确认 Cursor/控制器均退出且恢复 journal 已清理后，本机 EXE 更新至本构建，SHA-256 与发布包相同；保留原快捷方式目标、用户数据及旧 EXE 备份。没有启用真实订阅缓存注入。


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
