# G1a：模拟收发核心开发与运行

- 日期：2026-09-28
- 范围：G1 的第一个可运行切片，不是完整 G1 Freeze。
- 关联：[设计基线](../design/BASELINE.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md) · [矩阵](../adapters/CAPABILITY_MATRIX.md)
- 实际结果：[G1a 本地验收回执](../acceptance/receipts/2026-09-28-g1a-simulation.md)，包含代码提交与未覆盖项。

> 本文记录 G1a 切片及其当时边界，不作为整个仓库的最新功能表。后续已增加 [G1b HTTP 回复与回执补偿](G1_HTTP_PROVIDER.md)；G1a 的既有回执和 48 项测试口径保持不变。

## 1. 现在可以运行什么

仓库包含 Rust workspace：`foxbot-core` 是共享领域与持久化库，`foxbot-sim` 是离线合成消息 CLI。可执行“合成消息 → 固定回复服务 → 持久化发件箱 → 模拟写入/发送 → 效果核对”，并通过真实子进程退出测试恢复边界。

没有真实模型连接、API 密钥读取、系统截屏、OCR 推理、聊天客户端访问或原生输入能力。模拟发送只是写入一个专用合成事件文件，不会给微信、QQ、飞书或 X 发消息。所有原生适配器继续为 PLANNED / NOT_RUN。

## 2. 本地运行

需要已安装 Rust 工具链和本机 C 编译环境（bundled SQLite 需要编译）。`Cargo.lock` 固定依赖解析。manifest 的 Rust 下限为 1.89，但本轮实测工具链和平台以验收回执为准；尚未完成最低版本及 Windows/Android 编译矩阵。

从仓库根目录执行：

```bash
cargo run --locked -p foxbot-sim -- demo .foxbot-sim/g1a
cargo run --locked -p foxbot-sim -- demo .foxbot-sim/g1a
cargo run --locked -p foxbot-sim -- inspect .foxbot-sim/g1a
```

首次使用一个全新目录：历史只建上下文，新消息触发一次回复；两个来源对同一条合成消息的观察不重复执行。第二次使用相同目录：历史和新消息都已经见过，本次 provider/send 调用均为 0，已有模拟输出仍只有一条。

CLI 只打印计数、状态和随机任务 ID，不打印聊天正文。演示时间为确定性的合成时钟，不是性能数据。需要重新体验首次运行时，选择另一个新的状态目录；不要为了演示覆盖已有运行账本。

若终端或 Runner 找不到 cargo，可先在当前终端设置 `export PATH="$HOME/.cargo/bin:$PATH"`，或直接调用已有的 `$HOME/.cargo/bin/cargo`。不需要修改系统级 PATH 或重新安装工具链。

```bash
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
python3 scripts/check_docs.py
```

测试使用独立临时目录。进程测试在指定检查点终止子进程并等待退出；清理目录前确认进程已结束，不根据 IPC 不可达推断死亡。

## 3. 数据与执行边界

状态目录保存 `owner.lock`、`ledger.sqlite3` 及 SQLite 辅助文件；CLI 另保存仅包含模拟 action ID 的 `synthetic-outgoing.jsonl`。不要把状态目录提交到 Git；默认目录和数据库模式已加入 .gitignore。

**这里只允许合成数据。** SQLite 正文没有应用层加密，也未接系统凭据库、保留清理策略或生产级文件访问策略。Unix 下创建目录 0700、文件 0600，拒绝过宽权限和直接符号链接；这不等于加密，不宣称防御同用户恶意进程或全部路径竞态。Windows ACL 尚未验收。

数据库校验 application_id 与 schema_version，不修改不认识的数据库。开发中间版本的账本不保证迁移，不能当正式数据格式。该版本无生产密钥配置和联网代码。

## 4. 核心实现

### 身份、消息和队列

ConversationKey 由设备、应用实例、账号绑定和会话组成；标题仅用于展示。Observation 另带 identity_epoch，旧 epoch 的迟到观察拒收；消息去重和上下文查询同样按身份版本隔离。服务配置或身份版本变化旋转对外的匿名会话引用，旧任务失效。

canonical_id 是受信任适配器提供的已确认消息身份，不是正文哈希。来自多个来源的确定等价消息可以合并；缺身份或冲突文本保留 AMBIGUOUS，不猜测谁说了什么。真实截图/UI 树中的跨帧关联算法尚未实现。

初始化历史仅进入上下文；同一文字的两条独立消息保留。短消息按 quiet_ms 合并并由 max_wait_ms 限制最长等待，队列与批次有上限。批次上下文不会包含晚于本批次最后一条输入的消息，避免提前回答后续批次。上下文只表示有限观察片段，不保证完整聊天历史。

同一会话只允许一个活跃任务。新输入使未产生副作用的旧任务失效并合并；新观察到的己方消息使旧待回复输入失效，不重新回复已人工处理的旧批次。可靠人工接管事件由宿主调用 pause；本层没有操作系统级输入监听。

### 回复服务

ReplyProvider 是同步模拟接口。Custom 不追加 system prompt；Generic 只使用明确配置的提示词。接受 reply/no_reply/handoff，handoff 暂停会话。严格关联 request_id 和输入事件，不允许结果增加本地路由字段。空白、过长、控制字符、半截或错误 JSON 均不会自动修复后发送。

`generate_once` 是确定性演示便捷函数；真正的异步、网络超时和取消应使用分离的 begin_reply / accept_reply 生命周期，并在下一阶段增加调度器与时钟。现在没有 HTTP、SSE、模型目录、凭据管理或 service_managed 历史提交/取消协议，不宣称已经接通任意 AI 服务。

### 发件箱与恢复

PREPARED 在执行前持久化；写入动作之前提交 EXECUTING。检查账号、身份、会话、窗口、编辑器、布局、权限、用户输入和草稿；填入后再次读回，同步观察发现新消息或布局变化即停止。

通道接收为 SUBMITTED，匹配的新己方输出为 VERIFIED_OUTGOING；后者不是平台送达或已读。任何可能产生副作用后的异常都归 UNKNOWN，不自动改用另一通道重发。reconcile 只读核对，缺少输出不等于证明从未发送。

进程恢复时 EXECUTING/SUBMITTED 转 UNKNOWN；GENERATING 转 ERROR，不盲目重新请求可能有状态的服务。PREPARED 可经新鲜检查继续。UNKNOWN 阻塞对应会话的新回复，直到核对得到确定结果；现在没有自动解除未知状态的管理 UI。

持有状态目录锁直到 Runtime 释放，同一目录的两个进程不能并行拥有执行权；Rust 可变借用与同步通道调用串行化本 Runtime 的动作。**不同目录、不同设备不受这个锁约束**。以后统一设备级执行器之前不得声称完成跨实例或跨设备互斥。

## 5. 测试映射（覆盖范围，不是完整案例 PASS）

| 清单 | G1a 的可执行证据 | 尚未覆盖的部分 |
| --- | --- | --- |
| CORE-01、CORE-02、CORE-03、CORE-04 | 账号/身份版本隔离、历史与重启、确定性来源去重、重复短文本、冲突观察 | 真实客户端身份与无稳定 ID 的归并 |
| CORE-05、CORE-06 | 静默/最大等待、背压、批次上下文、迟到响应、配置变化、过期结果 | 真正的异步运行队列及跨会话公平调度 |
| CORE-07、CORE-08 | 同状态目录的进程锁、PREPARED 恢复、发送前后杀进程、UNKNOWN 不重发 | 不同目录的全设备执行权、分布式互斥 |
| AI-01、AI-03、AI-04、AI-07 | 固定 provider 捕获请求、no_reply/handoff、JSON/长度/关联检查、错误不自动重试 | HTTP 认证/限流、SSE、供应商实际协议 |
| AI-02、AI-05、AI-06 | 仅本地匿名会话配置隔离；未实现完整用例 | 服务端有状态历史、幂等请求、生成/发送分离、回执补偿队列 |
| TX-01、TX-02、TX-03、TX-04、TX-05 | mock 目标/草稿/输入法/布局/消息变化检查，读回失败不发送 | 原生焦点、真实输入法和剪贴板、应用发送快捷键 |
| TX-06、TX-07 | 杀进程及模拟外部日志证明；错回执/仅提交/缺回执不同状态 | 平台真实消息关联、失败图标、送达证明 |
| TX-08 | 己方消息不触发、持久化尝试上限 | 时间窗口限流、完整机器人循环/费用预算熔断 |
| GR-01、GR-02、GR-03 | 已标注消息保留 sender/mention/非文本完整性 | 从真实界面识别发言人、真实提及和复杂引用 |
| OC、AD、MC、WI、NW、RL 分组 | NOT_RUN | OCR、原生采集、真实收发、多会话值守、分发与持续运行 |

对应测试源码位于 `crates/foxbot-core/tests/runtime.rs` 和 `crates/foxbot-sim/tests/process_recovery.rs`。一个测试内的多个输入变化不虚增测试数量；进程测试不是实际聊天客户端测试。

## 6. 下一项：G1b，而不是直接宣布 G1 Freeze

补真实协议适配器与可控本地 HTTP 测试服务，定义 client_managed / service_managed 状态与提交/取消、幂等和回执补偿；引入可取消的异步调度、明确的时钟/过期语义和受控重试。随后完善资源、存储保护与设备执行权，再分别进入 G2 平台探针。

G1a 没有桌面 GUI 或 APK，也没有选择 FoxBot 自身许可证。依赖通过 Cargo.lock 固定；首次实现没有复制 Jev 源码或模型权重。

## 7. API 依据

SQLite 事务及 Rust API 使用 [rusqlite 文档](https://docs.rs/rusqlite/0.40.2/rusqlite/)；本地文件锁使用 [Rust std::fs::File](https://doc.rust-lang.org/std/fs/struct.File.html)。具体通过环境以独立验收回执为准，不由 API 文档推导跨平台已验证。
