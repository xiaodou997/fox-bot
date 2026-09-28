# G1a 模拟收发与恢复验收回执

- receipt_id：FB-G1A-SIM-20260928
- 日期：2026-09-28
- 执行环境：用户已连接的本地 WebCodex Runner，Darwin arm64。
- Rust：`rustc 1.98.1 (48a229cea 2026-09-01)`。
- 被测代码提交：`18914978cae2bf7002932957a9925eff1aabd559`。
- 开发分支：`feat/g1-simulated-runtime`，基于文档提交 `113e91347ad2ec25ecb6980404ffdf3a942b3414`。
- 执行者：本轮开发助手；未取得独立测试人员签名。
- 结论：**G1a 模拟切片通过本回执所列本地检查；完整 G1 尚未冻结，G2–G5 未运行。**
- 关联：[运行说明与案例映射](../../development/G1_RUNTIME.md) · [验收清单](../ACCEPTANCE_CHECKLIST.md) · [适配矩阵](../../adapters/CAPABILITY_MATRIX.md)

## 1. 实际执行

命令均在实际 checkout 中运行；不是把代码或说明发送给另一个 Agent 后推测结果。Runner 的 PATH 没有包含 `.cargo/bin`，本轮使用已有工具链的绝对路径调用 cargo，没有安装或替换系统工具链。

| 检查 | 真实结果 | 范围 |
| --- | --- | --- |
| `cargo test --workspace --all-targets --locked` | PASS：43 项核心集成测试＋5 项子进程测试，0 failed，0 ignored | 代码提交后再次完整运行；无原生适配器测试 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS，零警告 | 最终代码格式化后执行；此后未修改 Rust 源码 |
| `cargo fmt --all -- --check` | PASS | Rust 格式检查 |
| `git diff --cached --check` | PASS | 代码提交前暂存区检查 |
| `python3 scripts/check_docs.py` | PASS | 对代码提交内容的实际 checkout 检查：9 份 Markdown、40 处本地文件链接、2 份 JSON、61 个唯一案例编号，无错误 |
| workspace_hygiene_check | 仅报告本轮预期的 tracked 修改；无其他路径名风险发现 | 不读取文件正文，不是专用密钥扫描或安全审计 |

最后一次代码提交上的测试输出：核心测试 43 passed，运行 0.14 s；子进程测试 5 passed，运行 1.95 s。此前多次开发迭代的耗时不同；这里不将时间加总为稳定性验收，也不把编译时间作为处理消息延迟。

本回执与导航在代码检查后新增；以上文档计数固定对应被测代码提交，不把新增回执偷偷计入先前结果。

## 2. 演示复现

在新建的 `.foxbot-sim/g1a` 状态目录中运行两次：

```bash
cargo run --locked -p foxbot-sim -- demo .foxbot-sim/g1a
cargo run --locked -p foxbot-sim -- demo .foxbot-sim/g1a
```

| 字段 | 首次 | 同目录再次运行 |
| --- | --- | --- |
| messages | 2（历史＋新消息） | 2 |
| tasks | 1 | 1 |
| provider_calls_this_run | 1 | 0 |
| send_calls_this_run | 1 | 0 |
| synthetic_outgoing_count | 1 | 1 |
| 动作状态 | VERIFIED_OUTGOING | VERIFIED_OUTGOING |

VERIFIED_OUTGOING 只表示匹配的模拟输出已被观察到，不表示真实平台送达或已读。模拟外部日志只写 action ID，没有网络和原生输入行为。

本机最初开发时还运行过默认 `.foxbot-sim` 路径，其早期数据库未包含最终 schema 标识。该目录属于忽略的开发数据，不作迁移承诺、不覆盖它；最终说明使用独立的 `.foxbot-sim/g1a` 路径。

## 3. 进程级故障证据

`process_recovery.rs` 由测试进程启动实际 foxbot-sim 子进程，等待明确检查点后终止并 wait 确认退出，再打开数据库核对。

| 场景 | 实际断言 |
| --- | --- |
| 第二进程争用同一状态目录 | 被 Busy 拒绝；持锁子进程确认退出后可重新获取 |
| EXECUTING 已落库、外部发送尚未发生时终止 | 恢复为 UNKNOWN；无模拟外部日志；dispatch-prepared 与缺证据核对均不发送 |
| 模拟外部日志已 sync、发送回执尚未保存时终止 | 恢复为 UNKNOWN；只读核对后 VERIFIED_OUTGOING；日志始终只有一条 |
| PREPARED 后退出并重新运行 | 重新检查模拟目标和草稿后才发送 |
| 连续运行演示 | 第二次无新增 provider/send 调用 |

以上不是只在函数里修改状态的“假崩溃”测试，但外部系统仍是可控模拟日志，不能推导为真实微信、QQ、飞书或 X 已完成故障恢复。

## 4. 开发过程中发现并修复

首次编译遇到 rusqlite 的 SQL 整数转换约束，已改为明确的有符号存储与检查转换。第一轮测试因临时目录权限过宽而全部拒绝打开；已修正测试夹具为私有目录，没有放松存储权限检查。

补充回归后修复：分批任务错误带入后续批次上下文；身份 epoch 变化后旧观察、旧历史与重用消息 ID 的隔离；模型结果额外路由字段拒绝；发件箱目标与记录身份不一致拒绝；新观察到人工己方回复后不再重排已处理旧批次；过期 READY 任务标记为终态。Clippy 检出的分支写法及子进程检查点 EOF 处理也已修正。

发布前仍需要独立代码审查；本回执不宣称不存在其他缺陷。

## 5. 覆盖边界和未运行项

完整映射在 G1a 开发说明第 5 节。本回执证明部分 CORE、AI、TX、GR 的模拟子场景；**48 是 Rust 测试数量，不是 48 项完整产品验收，更不是清单 61 项全部通过。**

未实现或未验收：有状态业务服务的历史提交/取消和补偿、真实 HTTP/SSE/凭据、后台异步调度与截止时间、全设备执行权和跨设备排他、生产存储加密与清理、全套限流/预算、真实 OCR 和 UI 树归并、通知回复、原生窗口/输入法/剪贴板、应用收发、导航、桌面界面、APK、签名与升级。

锁只覆盖同一个规范化状态目录；不同目录和另一台设备不由此保证排他。G1a 是同步模拟器，不能承诺真实阻塞通道能随时取消。模型取消与外部发送回执也未接服务端。

没有进行 30 分钟交互 smoke、4 小时持续运行或隔夜测试。没有安装或操作真实聊天软件，没有使用真实账号/密钥/聊天数据，没有下载或运行 OCR 权重。原生适配矩阵全部保持 PLANNED / NOT_RUN。

## 6. 提交与后续

代码保存在本地开发分支；本回执单独提交，因此回执提交号不作为被测程序提交号。没有推送、合入远端 main、创建 Release 或发布安装包。

下一项为 G1b：真实协议适配与本地 HTTP 测试桩、异步调度，以及 service_managed 会话/幂等/发送事实补偿。完成相应模拟门禁后再决定 G1 Freeze 和 G2 平台只读探针。
