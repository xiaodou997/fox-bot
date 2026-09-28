# G1c：持续宿主、执行权与安全配置

- 日期：2026-09-28
- 范围：G1c 开发增量；不是完整 G1 Freeze，不是已具备真实聊天值守能力的产品发布。
- 前置：[G1a](G1_RUNTIME.md) · [G1b](G1_HTTP_PROVIDER.md)
- 关联：[基线](../design/BASELINE.md) · [适配矩阵](../adapters/CAPABILITY_MATRIX.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)

## 1. 本轮可以运行什么

新增 `foxbot-host` 库和命令行程序：持续接收合成消息，通过实际 HTTP 取得回复，按会话排队，模拟写入/发送，自动处理独立服务回执队列。一个进程可处理多个已配置的合成会话，不再每条消息启动一次命令。

本轮不读取真实聊天、不截图、不执行 OCR、不控制第三方窗口、不发送真实消息。这里的多会话测试只验证宿主调度，不能作为 G4 原生客户端导航和值守通过的证据。所有首批原生适配器仍为 PLANNED / NOT_RUN。

## 2. 一键复现与测试

```bash
cargo build --locked -p foxbot-host
python3 scripts/host_smoke.py
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 scripts/check_docs.py
```

Runner 没有 cargo PATH 时使用 `$HOME/.cargo/bin/cargo`，不需要修改系统环境。`host_smoke.py` 创建随机 loopback 端口、两个合成会话与临时账本，验证历史不补发、重复观察、网络期间暂停/取消、恢复新消息、回执 503 补偿、显式停止以及重启重放。它不访问钥匙串，也不读取开发用 FOXBOT_HTTP_TOKEN。所有子进程退出后才删除临时账本，服务器和读线程也会退出。

默认构建启用 `encrypted-ledger`：rusqlite 的 bundled SQLCipher 与 vendored OpenSSL 会参与编译；首次构建成本和包体不能与之前仅 bundled SQLite 混为一谈。精确依赖由 Cargo.lock 固定，当前实测平台看独立回执。取消该 feature 时仍可运行合成明文测试，但受保护入口必须拒绝，不可悄悄变成普通 SQLite。

可单独验证关闭加密功能的失败边界：

```bash
cargo test --locked -p foxbot-host --no-default-features --lib tests::disabled_cipher_feature_rejects_protected_entry_without_creating_plaintext -- --exact
```

## 3. 持续宿主命令行

[合成配置示例](../../examples/host-synthetic.json)中的 8787 是占位端口，不自动启动服务。一键 smoke 会自行替换端口。针对已经启动的本地测试服务：

```bash
cargo run --locked -p foxbot-host -- run examples/host-synthetic.json .foxbot-sim/g1c --allow-network --allow-plaintext-synthetic
```

宿主初始状态必定为 PAUSED。stdin 接受逐行 JSON，以下各行分别提交，不要把整个代码块作为一个 JSON：

```json
{"command":"status"}
```

```json
{"command":"resume"}
```

```json
{"command":"message","session":0,"id":"synthetic-new-001","text":"仅供合成测试：如何安装？"}
```

```json
{"command":"pause"}
```

```json
{"command":"stop"}
```

`session` 是配置 bindings 中的下标；CLI 用该绑定构造身份，不接受消息里的任意窗口/账号路由。`historical:true` 可用于模拟初始化观察。重复同一 id 与内容不会重复生成。未知字段、超长输入或不合法命令被拒绝，错误不回显正文。

状态输出只含计数、阶段以及 `synthetic_only=true`、`native_chat_operations=0`。停止命令在 stdin 仍然打开时也会使宿主退出；EOF 与 Ctrl-C 同样停止。stdio 入口只用于本地开发，不是认证过的远程管理 API。

## 4. 暂停、取消、重启与队列

| 行为 | 实际规则 |
| --- | --- |
| 新进程启动 | 总是暂停，不根据配置中的 enabled 直接开始联网回复。旧未发送任务失效，可能已发送的 UNKNOWN 保留。 |
| 全局暂停 | 取消生成，持久化暂停，旧 READY/PREPARED 失效；不会取消已经发生的外部发送。 |
| 暂停期间观察 | 只作为上下文基线，不建立恢复后的待发积压。 |
| 暂停前排队、恢复后才被读取的观察 | 控制代际发生变化，仍按历史处理，不能因队列延迟变成新消息。 |
| 恢复 | 恢复已授权的新消息处理；不自动撤销某会话的 handoff/禁用状态。 |
| HTTP 完成 | 使用完成时的新时钟检查任务/配置版本；取消和过期结果不能写入发件箱。 |
| 停止 | 优先取消 HTTP；有界等待作业结束；剩余任务 abort 后 join，记录取消/延期回执，再释放执行权。 |
| 恢复后的发送结果未知 | 不盲目重发；当前 CLI 没有自动解决所有 UNKNOWN 的管理入口。 |

宿主暂停是独立于 Binding.enabled 的持久化控制。单个会话已经转人工或因执行保护停用，不能靠重启或全局 resume 偷偷重新启用；目前其显式重新授权入口仍需后续管理层实现，不建议直接编辑数据库。

数据队列容量 64、控制队列容量 8。停止使用独立 CancellationToken，避免排在数据积压后面。单次命令行输入最多 32 KiB。配置最多 32 个会话，全局生成作业上限 1–8，同一会话同时最多一个活跃生成，采用轮转顺序给不同会话机会。每次 tick 至多执行一个模拟发送；tick 10–1000ms，并跳过错过的 tick，避免补偿式忙循环。

回执另有一个 worker，共用该 HTTP 实例的网络并发限制。暂停仍允许补传已经授权的旧任务回执；它不能产生新的聊天发送。如果需要完全停止网络，请使用 stop，而不是把 pause 误认为网络总开关。

`shutdown_ms` 为网络作业退出的等待预算，不是任何原生系统调用都可以实时中断的保证。当前 MessageChannel 和 SQLite 操作同步执行；将来接入真实原生控制前必须再做有界工作线程/辅助进程、用户接管与超时验收。stdin 读取线程本身没有运行核心、凭据、网络或执行锁能力，CLI 退出时随进程结束。

## 5. 执行权范围

`DeviceOwner` 不再跟随账本目录。所有遵守此入口的宿主使用当前 OS 用户固定的本地数据目录：`dirs::data_local_dir()/FoxBot/execution-v1/device-owner.lock`。macOS 对应用户 Library/Application Support 下的位置。

同一用户启动第二个宿主，即使选择不同账本，也应在打开该账本前得到 Busy。锁持有至宿主退出；文件不在 Drop 中删除，避免 inode 替换造成两组互不冲突的锁。Unix 下核对 inode/device、硬链接数和权限，可检测意外替换锁文件。进程被终止后，后续进程可以重新获得该文件锁。

**这是同一用户下的协作进程互斥，不是跨设备、跨登录用户或抵抗同用户恶意进程的机制。** 旧 foxbot-sim / foxbot-http CLI 仍只是合成测试入口，不共享该设备锁；它们本来不具备原生发送能力。嵌入方必须持有统一执行权，不可换个自定义锁目录宣称全局排他。测试库内部使用隔离 scope，产品 CLI 不暴露覆盖路径参数。

## 6. 凭据与安全配置

配置只保存 CredentialRef，例如 `{"id":"test-ledger-v1"}`，不保存密钥、token 或任意钥匙串查询条件。引用 ID 限定字符和长度，Keychain service 固定为 `io.foxbot.credentials.v1`，用途区分 ledger 与 token。

目前 NativeCredentials 的已实现平台为 **macOS**，调用系统 Security.framework。其他平台明确返回 Unsupported，不使用环境变量/明文文件作为隐藏降级。库允许注入测试 CredentialStore 来验证缺失、锁定等失败路径；注入假实现通过不等于其他 OS 原生凭据已完成。

仅在用户显式创建新配置时，可运行以下命令生成随机 32-byte 账本密钥并存入自己的钥匙串条目；名称是示例，不是本轮替用户创建的长期密钥：

```bash
cargo run --locked -p foxbot-host -- init-key test-ledger-v1 --confirm-keychain-write
```

API token 的创建入口为 `set-token NAME --confirm-keychain-write`，秘密通过 stdin 读取，不放 argv、JSON 或仓库；输入须无换行等控制字符。已有同名凭据拒绝替换。不要把真实 token 写进 shell 历史或示例文件；当前仍不提供完整的凭据轮换 UI。

在一份完整配置中，把 storage 改为下面的对象，并去掉运行时的明文许可参数：

```json
{"kind":"protected","key":{"id":"test-ledger-v1"}}
```

有认证时，顶层 token 可以设为 `{"id":"test-provider-v1"}`。这是配置字段片段，不是独立可运行配置。加密账本名称与 token 名称分离；token 为 null 时不会查询已有用户凭据。合成明文入口必须同时使用明确许可，且不允许配置 token 引用。

钥匙串失败分为缺失、不可用、格式错误、已存在等固定错误；不会输出 API 原始错误正文或秘密。Secret 无 Debug/Serialize，并用 Zeroizing 清理本组件拥有的缓冲区。网络库、OS API 和 SQLite 内部可能存在自己的副本，不承诺整个进程内存不含密钥，不宣称防止运行中进程被调试或内存读取。

本机原生探针有单独的显式入口：

```bash
cargo run --locked -p foxbot-host -- keychain-smoke --allow-keychain-test
```

它仅使用随机名称新建 FoxBot 测试条目，验证存取相等、禁止覆盖、删除和删除后 Missing；不枚举或读取既有用户凭据。实际执行与删除结果见回执，不从代码存在推导真机通过。

## 7. 加密账本与迁移

`Runtime::open_encrypted(directory, &[u8;32])` 与 `open_simulation(directory)` 是独立入口。受保护模式先设置 SQLCipher key、核对 cipher_version 并读取 sqlite_master 验证，再进行 schema 操作；错误密钥不能触发新库覆盖。关闭加密 feature 时受保护入口直接失败。

本轮测试实际检查了数据库和 WAL 中不出现指定合成正文、正确密钥重开、错误密钥拒绝、密文篡改拒绝及明文库不被静默导入。该检查只证明这些边界案例，不是完整密码学/供应链审计。

schema 新增 host_control，版本从 2 增量升级到 3，同时兼容已知版本 1 的升级。结构升级不等于明文转加密：旧合成目录仍是明文；不会自动迁移、删除或加密旧文件。不要把新的二进制交给旧程序继续写版本 3 数据库。

本轮尚未完成密钥轮换、丢钥恢复、明文导入/安全删除、备份、保留清理、容量预算与生产级路径/Windows ACL 验收。缺钥时停止，不自动创建一个新密钥假装旧数据恢复。加密保护静态账本，不防运行中已获权限的同用户恶意程序，不替代系统账户保护。现有运行演示仍只使用合成内容。

## 8. 验收映射与未覆盖项

| 清单范围 | G1c 可执行证据 | 仍需独立验收 |
| --- | --- | --- |
| CORE-02、CORE-05、CORE-06 | 启动暂停、代际基线、轮转/并发界限、新消息取消、晚到结果 | 真实通知/截图的消息发现与去重 |
| CORE-07、CORE-08 | 跨状态目录的同用户进程锁、杀进程释放、重启暂停与旧事件不重放 | 跨用户/跨设备、设备级原生执行工作进程 |
| AI-06、TX-06、TX-07 | 连续宿主自动补传回执、不再发聊天消息、未知状态保留 | 真实平台送达/失败关联 |
| TX-01、TX-02、NW-03 | 继承模拟前后校验，显式暂停/停止，不自动撤销 handoff | 实际焦点、输入法、窗口导航、人机抢占 |
| RL-01、RL-04 | schema 3 增量升级、命名凭据/失败关闭、SQLCipher 合成测试 | 发布签名、全生命周期迁移/清理、其他平台凭据与 ACL |
| RL-02 | 短时测试与 smoke 的实际时长 | 30 分钟、4 小时或隔夜持续验收及资源预算，均未进行 |

新增测试在 `crates/foxbot-host/src/tests.rs` 与 `crates/foxbot-host/tests/process_control.rs`；HTTP fixture 复用本仓库原有测试模块，不是复制外部业务服务实现。数量与被测 SHA 由独立回执记录。

下一步为 **G1 集成收口与 G2 macOS 只读探针准备**：先对照清单把尚未满足的发布阻塞项列清，再实现只读原生能力报告；不启用真实发送。系统凭据与加密成功不等于整个 G1 或产品已经冻结。长时间运行、保留清理、恢复管理、其他平台原生安全入口和实际客户端身份识别仍有独立边界。

## 9. API 与依赖依据

- [rusqlite 0.40.2 features](https://docs.rs/crate/rusqlite/0.40.2/features)：bundled-sqlcipher-vendored-openssl。
- [SQLCipher API](https://www.zetetic.net/sqlcipher/sqlcipher-api/)：设置 key 与实际读库验证、加密连接行为。
- [Security.framework Rust bindings](https://docs.rs/security-framework/3.7.0/security_framework/passwords/index.html)：macOS 命名密码条目接口。
- [Rust File locking](https://doc.rust-lang.org/std/fs/struct.File.html)：持有文件锁，不以删 lock 文件代替释放。
- [Tokio JoinSet](https://docs.rs/tokio/latest/tokio/task/struct.JoinSet.html)：中止任务和等待退出分别处理。

这些资料是实现依据，不是所有目标 OS 已通过的证明。依赖锁定与分发许可记录见[来源审计](../references/UPSTREAM_AUDIT.md)。
