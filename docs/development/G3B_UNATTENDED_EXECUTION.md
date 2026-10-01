# G3b：无人值守独占执行调整

- 日期：2026-10-01
- 决策来源：用户明确实际运行不会人工操作聊天软件；仅开发期间需要交还使用权。
- 范围：G3b 执行契约、Rust/Swift 门禁、回归和当前文档；不新增真实发送动作。
- 关联：[设计基线](../design/BASELINE.md) · [G3b Gate](G3B_SAFE_SEND_GATE.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)

## 1. 产品契约

AUTO_REPLY 按 **UNATTENDED_EXCLUSIVE（无人值守、FoxBot 独占聊天界面）** 设计。用户授权账号和会话范围并启动后，正常流程是读取新消息、取得 AI 完整回复、填入目标输入框、核对内容、发送及核对结果。

本期不支持人工与自动化同时编辑同一聊天界面，不做人工键鼠活动识别、输入法组合态证明或自动让权仲裁。独占是部署和使用约定，不是要求软件再证明“当前绝对无人操作”的新门禁，也不新增强制独占探测或每条回复人工确认。

ASSISTED / AUTO_SUGGEST 保留原有含义；它们不承诺人机同时操作目标输入框。GUI 进程互斥、账号/会话授权、权限和显式暂停仍然有效。

## 2. 移除什么，保留什么

| 项目 | 调整 |
| --- | --- |
| `composition_verified` / composing / user_active | 从 `LiveTarget` 和 Core Gate 移除，不再要求平台提供，也不伪造 true/false。 |
| IME、候选窗、最近键鼠活动 | 从 native send-gate 的事实和阻断条件移除；主路径不再调用 IME probe 或活动计时。 |
| IME Evidence Spike | 归档为独立诊断实验，不再阻塞 G3c，不扩大成兼容性专项。 |
| 目标和内容正确性 | 保留账号会话身份、surface/layout、前台与权限、写前草稿状态、写后内容匹配。 |
| 持久化和防重复 | 保留 revision、action state、attempt budget、DeviceOwner、UNKNOWN 对账及禁止盲重发。 |
| 开发期间交接 | 使用现有 pause/stop/resume；不新增自动干扰检测。 |

残留草稿或不可读输入区仍不能直接拼接/发送：即使没有人工使用，也可能是前次中断留下的内容。检查服务于发送正确性，不是恢复“人工编辑支持”。不在本轮增加自动清空或覆盖策略。

## 3. 实现与协议

Rust 的 `LiveTarget`、`accepts()` 和 Runtime 双门禁使用一致的无人值守契约；删除旧 IME/人工活动字段及对应 blocker。`LiveTarget` 不是持久化对象，本轮不改数据库 schema、Reply Provider 协议或 outbox 内容。

Swift 的 `NativeSendGateFacts` 只保留 capture、frontmost、两次 surface 稳定、app-session、conversation resolved/match、draft match 七项事实。任一缺失仍阻断。最终 Gate 使用捕获后重新查询的前台状态，而不是沿用捕获前快照。

`foxbot-macos-send-gate` 的公开报告升级到 `foxbot.macos-send-gate.v2`，显式报告 `execution_model=UNATTENDED_EXCLUSIVE`、`input_state_policy=NOT_REQUIRED`，移除原 IME/人工活动字段。NOT_REQUIRED 是契约，不是已经探测到 SAFE；旧报告不得混当新版结果。该程序仍只读，`write_operations=0`、`send_operations=0`。

独立 `foxbot-macos-ime-evidence` 工具和实验单测保留以便追溯，不在正常填入/发送路径调用。

## 4. 开发期间交接

测试人员需要操作聊天软件时，先 pause 或 stop，确认当前执行已结束后再操作；完成后把测试会话交还 FoxBot，显式 resume 或重新启动测试。暂停不会被解释成撤销已经发生的动作，也不承诺同步原生调用可瞬时中断。

沿用现有暂停语义：旧待发任务失效，恢复不复活旧回复；新的有效消息才建立新任务。暂停期间的历史不批量补发。无需准备“用户边选词机器人边填入”的测试。

## 5. 回归范围

固定代码提交 `aafbb76` 已完成 13/13 集成检查，Rust 137 / Python 61 / Swift 97 全部通过；完整证据与未执行范围见[本次回执](../acceptance/receipts/2026-10-01-g3b-unattended-execution.md)。

保留并验证无人值守正常路径、无副作用 preview、每项目标/权限/内容校验、写入未生效/截断、surface 变化、revision 过期、显式暂停恢复、DeviceOwner 和 UNKNOWN 不重发。

Swift `testUnattendedReadyNeedsNoInputStateEvidence` 验证 READY 不再需要 IME 事实；`testEveryExecutionFactStillBlocksIndependently` 验证保留的七项条件分别有效。Rust `developer_pause_invalidates_old_action_and_resume_only_allows_fresh_work` 验证开发交接不重发旧任务。

TX-02 / TX-04 / TX-05 / WI-02 / NW-03 按独占契约修订；删除人机共编和输入法组合态要求，但保留残留草稿、布局/内容错误和显式暂停的测试。历史回执保留当时的失败与观察，不改写为 PASS。

## 6. 下一步：G3c 当前会话真实发送

不再等待 IME 证据，也不因为 IME 不可读而改变目标微信版本/通道或移除一期自动回复。

下一开发增量是 G3c-1：在已绑定的当前测试会话中，接通持久化待发任务 → 平台回填 → 内容复核 → 应用专属单次发送 → 匹配新己方消息的回执。先用固定测试回复隔离模型变量，再接真实回复服务；群聊、多会话导航另行推进。

需直接解决的实际工作是测试级 Draft Writer 到 Runtime 的适配、支持文本的回读和一次发送后的结果关联；不是再新增一个无法取得的绝对安全证明。真实发送仍需对应实现和指定测试会话验收，本次门禁调整不等于 C07/C08 已完成。
