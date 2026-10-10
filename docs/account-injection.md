# 本地订阅展示状态：选择性注入

本功能默认关闭，只在明确同意后临时设置两个本地缓存字段：`cursorAuth/stripeMembershipType=ultra`、`cursorAuth/stripeSubscriptionStatus=active`。它不更换账号、不授予官方订阅权限、不改变官方权限响应。官方模型及云端服务仍受真实账号权限限制。

此前四项凭据切换方案已废弃。核心没有凭据输入接口，不读写 accessToken、refreshToken、cachedEmail、cachedSignUpType。`stripeMembershipAuthId` 仅作为只读相等护栏保存在加密 journal 中，任何情况下都不写入。

## 固定上游审查与当前边界

审查基于 `leookun/cursor-byok` 提交 `7ee68c2b7fef66a0e0279273d037d23fbc2f11ad` 的 `server/src/local_app/account.rs`。

| 字段 | 上游来源 | 当前行为 |
| --- | --- | --- |
| `cursorAuth/accessToken` / `refreshToken` | 本地拼接占位 JWT，`sub=cursor-local-user`、`iss=cursor-client` | 不读取或写入；官方转发边界仍拒绝已知占位身份 |
| `cursorAuth/cachedEmail` | 固定 `cursor@ai.com` | 不读取或写入 |
| `cursorAuth/cachedSignUpType` | 固定 `Google` | 不读取或写入 |
| `cursorAuth/stripeMembershipAuthId` | 固定 `cursor-local-user` | 只读原值快照，用于识别期间账号切换；不写入 |
| `cursorAuth/stripeMembershipType` | 固定 `ultra` | 经同意临时写入 `ultra`，退出时按值比较恢复 |
| `cursorAuth/stripeSubscriptionStatus` | 固定 `active` | 经同意临时写入 `active`，退出时按值比较恢复 |

Cursor 会把这两项缓存镜像到 `ItemTable` 的 `src.vs.platform.reactivestorage.browser.reactiveStorageServiceImpl.persistentStorage.applicationUser` JSON 对象中的 `membershipType` / `subscriptionStatus` 叶字段。核心在注入前仅保存这两个叶字段的原值；注入阶段不写该 JSON 行。恢复时仅处理仍等于注入值的叶字段，保留其他当前 JSON 数据和 TEXT/BLOB 类型，不用旧对象覆盖当前对象。

## 接口与生命周期

```text
server/src/local_app/
├── account.rs      # 显式路径、两字段注入、派生叶恢复与账号护栏
├── secrets.rs      # Windows CurrentUser DPAPI
└── atomic_file.rs  # 同目录 journal 原子刷盘替换
server/tests/
└── account_injection.rs # 临时 SQLite、合成数据回归
```

- `inject(database, journal, consent)`：未同意则拒绝。打开已有 SQLite，在 `BEGIN IMMEDIATE` 下读取两个原值、AuthId 护栏和两个派生叶原值；Prepared journal 经 CurrentUser DPAPI 加密并原子刷盘后，只提交两个 stripe 字段，再标记 Active。
- `restore(database, journal)`：关闭、退出和启动恢复共用。AuthId 未变化时，逐字段比较恢复仍等于注入值的 stripe 与派生叶；不存在的原字段恢复为不存在。期间第三方写入不同值、删除字段或修改无关 JSON 数据均保留。
- AuthId 变化时，跳过全部 stripe 和派生字段恢复，保留新账号值，返回 `account_changed=true` 与 `preserved_fields` 冲突字段名。完成此保留处理后清除 journal，避免未来换回旧账号时误恢复旧值。AuthId 原本和现在都缺失时无法据此识别账号切换，这是该护栏的范围限制。
- `pending(journal)`：检测待恢复记录，不打开 Cursor 数据库。
- `is_placeholder_token(token)`：官方请求边界复用的已知占位身份检测，不参与本地注入、不记录 token 内容。

调用方必须持有统一生命周期锁，在注入和恢复前确认 Cursor 完全退出。核心不停止进程、不发现真实 profile、不发起网络请求。实际启停和托盘退出流程由控制器集成。

Prepared 在数据库提交前后都可恢复；Restoring 在恢复提交后、journal 删除前可幂等重试。SQLite 写入或提交失败时整个事务回滚，journal 保留；解密、路径或删除失败也保留恢复记录。错误不转发 SQLite trigger 原始消息，避免敏感内容进入日志。第三方导致派生 JSON 无法解析时保留该行、报告冲突并恢复可安全处理的 stripe 字段。

原始 stripe 值区分不存在、TEXT、BLOB；其他类型在注入前拒绝。不复制或覆盖整个数据库，不迁移 schema，不重建缺失数据库。数据库及 SQLite sidecar、journal 沿用绝对路径与 reparse 校验，journal 不能指定为数据库或 sidecar。

## 验证范围

隔离回归仅使用临时数据库和合成元数据。覆盖默认关闭、两字段范围、凭据字段不变、DPAPI、WAL、TEXT/BLOB/不存在、派生叶镜像、其他 JSON 改动保留、账号切换护栏、第三方修改、损坏 JSON、Prepared/Restoring 幂等、恢复失败回滚、损坏 journal 和不安全路径。

本地注入值不等于真实订阅。真实 Cursor 的镜像行为、启停恢复和官方权限边界需要分别验收；隔离测试不能宣称官方模型权限或真实账号闭环通过。本轮实际命令和结果由 `docs/VALIDATION.md` 统一登记。
