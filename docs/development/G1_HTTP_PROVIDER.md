# G1b：HTTP 回复协议与持久化回执补偿

- 日期：2026-09-28
- 范围：G1b 工程增量；不是完整 G1 Freeze，不是原生聊天适配验收。
- 前置：[G1a 运行核心](G1_RUNTIME.md) · [设计基线](../design/BASELINE.md)
- 验收定义：[验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)
- 实际结果：[G1b 本地验收回执](../acceptance/receipts/2026-09-28-g1b-http.md)，被测源代码提交和未覆盖项独立记录。

## 1. 已实现的闭环

新增 `foxbot-http` crate 与同名命令行程序。网络请求由异步 reqwest/Tokio 执行，运行核心不绑定 HTTP；实际调用本机测试服务后，可以接受回复、准备模拟发件箱、模拟发送，并将效果回传给服务。这里的“真实 HTTP”指实际 TCP/HTTP 往返，不是已调用真实模型或真实聊天软件。

核心新增服务交换记录和独立回执队列，SQLite schema 从 1 增量升级到 2。原有消息与发件箱保留；不支持将版本 2 账本交给旧 G1a 程序继续写入。模拟账本仍未加密，只使用合成数据。

## 2. 两种传输协议与独立的提示词策略

| 协议 | 当前支持 | 不隐含支持 |
| --- | --- | --- |
| `chat_completions` | 明确 endpoint、model、Bearer token；非流式文字请求；解析单个完整 assistant 结果 | Responses、Anthropic、Gemini、工具执行、任意 SSE 或所有兼容厂商扩展 |
| `business_v1` | FoxBot 的结构化业务 envelope；reply/no_reply/handoff；client_managed 或按下述契约 service_managed | 任意旧服务字段自动推断、无协商的服务端历史追加 |

传输协议不决定是否添加角色提示词。已绑定知识库的服务即使使用 Chat Completions，也可使用 `ProviderProfile::Custom`，只发送 user 数据；只有宿主明确配置 `ProviderProfile::Generic` 才增加指定的 system prompt。CLI 默认 Custom。BusinessV1 不接受 Generic，避免默默丢掉宿主明确配置的提示词。

Chat Completions 的 user 内容是包含 `context`、`input_events`、`context_complete`、`user_request` 的 JSON 文本，保留发言人、内容类型、引用与完整性。聊天中的“system”等文字不会被提升为系统角色。输入事件可能也出现在上下文中，其 event_id 相同，服务应据字段语义区分上下文与当前输入。未实现只发送裸正文或任意字段映射的配置器。

响应必须有一个 index=0 的 choice、finish_reason=stop、assistant 字符串内容；截断、refusal、工具调用、错误结构不会成为待发送回复。BusinessV1 还严格校验请求、输入事件和服务会话引用。流式结果尚未接入，返回 SSE 会被拒绝，不发送半截文字。

## 3. 配置与本地复现

示例：[业务协议](../../examples/http-business.json) · [Chat Completions](../../examples/http-chat-completions.json)。它们是字段示例，8787 端口不会自动启动服务，example.invalid 也不是可调用供应商。下面的一键 smoke 自行创建随机本地端口、临时配置和状态目录，无需密钥。

```bash
cargo build --locked -p foxbot-http
python3 scripts/http_smoke.py
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 scripts/check_docs.py
```

找不到 cargo 时使用 `$HOME/.cargo/bin/cargo`。smoke 仅监听 127.0.0.1，移除子进程继承的 FOXBOT_HTTP_TOKEN，结束时停止服务并清理临时目录。它验证首次生成/模拟发送、同目录重放，以及回执首次 503 后独立补偿；不会打开聊天软件。

对已经自行配置好的测试服务，CLI 为：

```bash
cargo run --locked -p foxbot-http -- synthetic-run examples/http-business.json .foxbot-sim/g1b --allow-network
cargo run --locked -p foxbot-http -- inspect examples/http-business.json .foxbot-sim/g1b
cargo run --locked -p foxbot-http -- feedback examples/http-business.json .foxbot-sim/g1b --allow-network
```

生成与回执网络操作必须显式传 `--allow-network`。`inspect` 不发送 HTTP，但打开账本会执行既有恢复规则；不是对数据库的纯只读连接。`synthetic-run` 只使用内置合成消息和内存 MockChannel，同目录再次运行不重复处理已见事件。CLI 不自动派发重启后遗留的 PREPARED/UNKNOWN 动作；恢复工具仍按核心的独立校验语义处理。CLI 无定时后台循环，补偿命令只处理当前到期队列，失败记录退避后退出。

Bearer token 可由嵌入宿主在内存中传入；开发 CLI 仅从 `FOXBOT_HTTP_TOKEN` 环境变量读取。不要将密钥写入 JSON、命令参数、仓库或回执。环境变量是开发入口，不是已经实现的系统凭据库；本轮测试未读取或调用任何真实模型凭据。

配置分别限制单次请求时间、整个请求（包括排队/重试/读响应体）时间、尝试次数、响应字节数和同一服务实例的并发数。实例 clone 共用限流器，不同实例目前没有进程级总预算。

## 4. BusinessV1 的最低契约

生成请求字段由客户端构造：schema_version=0.1、request_id、idempotency_key、conversation_ref、session_revision、provider_profile_version、context_mode、input_events、context_complete，以及存在时的 user_request。client_managed 附有限 context；service_managed 不重复附整段上下文。

服务返回格式：

```json
{
  "schema_version": "0.1",
  "request_id": "request_example",
  "conversation_ref": "scope_example",
  "in_reply_to": ["event_example"],
  "complete": true,
  "outcome": {"result": "reply", "text": "合成回复"}
}
```

outcome 也可为 `{"result":"no_reply"}` 或 `{"result":"handoff","reason":"需要人工处理"}`。这些是业务结果，不需要第二个意图模型。接口路径由配置精确指定，不自动拼接路径或切换供应商。

service_managed 必须明确提供：

1. **幂等生成。** 同一 request_id/Idempotency-Key 加相同请求体只有一个逻辑生成；冲突内容不能覆盖原请求。请求响应丢失后可返回同一结果，不重复追加历史。
2. **生成暂存。** 生成回复时不把它当作已告知客户，也不提前提交尚可能取消的整个轮次。cancelled 可以丢弃该轮；重新提交被合并的输入事件不会重复进入已提交历史。
3. **持久化回执。** 按 receipt_id 去重，并按 request_id 的 revision 单调处理。先收到取消回执、后收到延迟的生成请求时，取消 tombstone 仍然生效；丢失 ACK 的重放返回同一确认。

这些字段为宿主声明的契约，代码无法凭一个布尔值自动证明第三方服务真的实现了幂等或暂存。只有用目标服务完成对应联调，才能宣称该服务有可靠自动回复能力。没有这种契约时，采用 client_managed 且关闭自动重试；不能强行包装成有状态可靠模式。

服务会话引用由本地匿名会话引用和服务语义配置生成，不由模型正文选择。尚未实现第三方任意 server_conversation_id 的映射或其历史修正接口。API key、endpoint/model、语义契约改变会改变服务作用域；旧凭据或目的地的回执不能转给新服务。宿主必须同时增加 core Binding 的 profile_version 来取消旧任务；CLI 对自身合成绑定执行这一操作。仅调超时/并发/响应大小，不改变回执作用域。

## 5. 异步生命周期与取消

宿主按 `begin → run.await → finish` 调用。begin 创建核心任务并持久化服务交换；run 不借用 Runtime，所以等待 HTTP 时宿主仍可采集新消息、暂停会话和修改配置；finish 使用完成时的新时间，并通过核心版本校验。

`CancellationToken` 可取消排队、读取响应和退避等待。成功响应已到达但尚未接受时的取消也优先。取消只是本地停止等待，不能声称服务端停止执行。宿主主动丢弃 future 后，应调用 cancel 收口；进程崩溃则由账本恢复 GENERATING→ERROR，再派生取消回执。

`RunClock` 用进程启动时的 epoch 加 Instant 已用时间；进程内不因系统时钟调整倒退，跨重启仍由核心拒绝早于任务创建的时间。此阶段提供可取消、受并发限制的 HTTP 作业，不是完整的公平多会话调度器、设备级全局执行锁或后台常驻守护进程。

## 6. 重试、未知结果与回执补偿

reqwest 的隐式重试关闭。生成只有在明确声明 idempotency_supported 时才进行最多三次的受控重试；每次使用相同 bytes 和 key。认证错误、格式错误、重定向、超长或不完整响应不重试。限流/服务暂时错误和传输故障按策略退避；Retry-After 目前只支持 delta-seconds，日期或畸形值保守停止，不缩短服务器要求的等待时间。

服务效果队列与聊天发件箱独立：

| 客户端事实 | 服务回执 | revision | 历史含义 |
| --- | --- | --- | --- |
| 可能已发生发送、只有通道提交、结果未知 | unknown | 1 | 暂不确认客户已收到，也不取消可能发出的回复 |
| 已观察匹配的己方输出 | observed_outgoing | 2 | 可提交实际发出记录，但不声称送达/已读 |
| 未发送任务失效、生成出错或取消 | cancelled | 2 | 丢弃暂存轮次，保留 tombstone 防止晚到生成恢复 |
| 明确不回复 / 交接人工 | no_reply / handoff | 2 | 提交该业务结果，不形成客户消息 |

反馈不携带聊天正文，包含 receipt_id、request_id、conversation_ref、revision、disposition、可选 action_id。ACK 示例：

```json
{
  "schema_version": "0.1",
  "receipt_id": "request_example:2",
  "revision": 2,
  "accepted": true
}
```

回执从核心任务/发件箱事实重建，因此“发送后尚未入回执队列就崩溃”不丢掉对账要求。发送回执前先持久化 IN_FLIGHT；重启恢复 PENDING，继续用相同 receipt_id。ACK 丢失可能重复传输回执，但绝不通过这个队列重新发客户消息。

未知回执 revision1 与最终 revision2 可乱序到服务；服务必须忽略低版本对高版本的覆盖。stateful 会话在 final revision2 获确认前不开始下一轮，防止历史超前。回执失败使用持久化退避，最多八次后 SUSPENDED；错误 ACK 也暂停，不假装已经同步。旧配置回执只能经匹配配置处理；跨凭据迁移、解除 SUSPENDED 的管理 UI 尚未实现，不能直接改数据库当正常恢复功能。

## 7. 安全和未覆盖范围

默认只接受 HTTPS 且保留正常证书校验。明文 HTTP 仅允许显式开启的数字 loopback 地址，不接受 localhost 解析或非本机地址；URL 内凭据、query、fragment 被拒绝。回执 endpoint 需与生成 endpoint 同源。重定向不跟随，不继承环境代理，不关闭 TLS 校验。后续代理设置需单独设计授权与测试，不能偷偷沿用全局代理。

错误类型不保留底层 URL、正文或认证 header；API key 不写账本。语义配置和凭据的不可逆指纹用于关联，不可用来证明身份安全；开发账本权限和 SHA 指纹不等于加密，也不防同用户恶意进程。

当前未完成系统凭据库、数据库加密和保留清理、TLS 证书故障 fixture、最低 Rust 版本和多平台构建、厂商真实服务验收、任意 SSE 与供应商协议、真实 OCR/窗口/输入、设备级单一执行权、持续值守或安装包。所有真实聊天适配器仍为 PLANNED / NOT_RUN。

## 8. 测试与门禁对应

| 清单范围 | 本轮证据 | 不可据此推导 |
| --- | --- | --- |
| AI-01、AI-03、AI-04 | 两种 HTTP 结构、无额外 prompt、结果类型、截断/工具/错关联拒绝 | 所有服务协议通用兼容 |
| AI-02、AI-05、AI-06 | 约定的 service_managed 暂存/幂等/取消、增量请求、乱序回执与重启补偿 | 未联调的服务也具备暂存历史契约 |
| AI-07、CORE-06 | 错误脱敏、无重定向/静默换供应商、限流、取消、晚到/过期结果 | 原生权限和所有网络安全场景已验收 |
| CORE-08、TX-06、TX-07 | HTTP 子进程被终止后的恢复、回执与聊天发送解耦、UNKNOWN不重发 | 任意 GUI 应用恰好发送一次 |

Rust 测试位置为 `crates/foxbot-http/tests/http_runtime.rs`、`tests/cli.rs`，同时保留 G1a 43＋5 项。测试服务本身只是合成协议 fixture，不可部署成正式知识库服务器。最终数量、被测提交与命令结果以独立验收回执为准，不把案例映射视为完整案例 PASS。

下一项 G1c：完善宿主任务调度/暂停与设备执行权、凭据与存储保护，再准备 G2 原生只读探针；不得以 HTTP 全绿替代这些门禁。已确定的软件范围和全自动模式保持不变。

## 9. 实现依据

采用 [reqwest ClientBuilder](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html) 的超时/重定向/重试配置、[Tokio CancellationToken](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html) 的取消机制，以及 [Chat API 参考](https://developers.openai.com/api/reference/resources/chat) 的请求/完整响应字段。依赖精确解析由 Cargo.lock 记录；本地 fixture 和实际编译测试是本次实现证据，不从在线文档推导目标设备或供应商已通过。
