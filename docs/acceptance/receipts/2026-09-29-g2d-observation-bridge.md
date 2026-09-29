# G2d Observation Bridge 本地验收回执

- 日期：2026-09-29
- 被测代码提交：`89d81649f8a6330081f76bca203b65ce6ad7de50`
- 分支：`feat/g2d-observation-bridge`
- 结论：**实现 / 合成 Runtime Bridge PASS；真实微信 Observation 验收 BLOCKED。**

## 固定提交门禁

- Rust：121 项通过
- cipher-disabled：1 项通过
- Python：49 项通过
- Swift：71 项通过
- `g2d-bridge-smoke`：通过
- source fingerprint：`9a911413bcafe57974df03979f8825fc71750cde79668efbca7ac5897a17a649` 前后一致
- external model requests：0
- native chat operations：0

## 合成端到端证据

固定提交 smoke：

~~~text
baseline.bridge_state = BASELINE
baseline.observations = 3
baseline.baselined    = 3

new.bridge_state      = NEW
new.observations      = 1
new.queued            = 1

repeat.bridge_state   = NO_CHANGE
repeat.observations   = 0

runtime.messages      = 4
runtime.tasks         = 0
runtime.ready         = 0
runtime.unresolved    = 0
~~~

额外 Rust 测试覆盖：PROVISIONAL/AMBIGUOUS 零 Runtime 写入；Backpressure 中途失败后重试复用相同 canonical id，已写 prefix 为 Duplicate，bridge cursor 仅在整批被 Runtime 接受后提交。

## 真实端状态

没有真实 accepted ground-truth，也没有为真实微信会话建立显式 Binding，因此不允许执行真实 Observation ingest。固定提交后续真实只读尝试遇到目标状态变化，worker 脱敏状态为 TARGET_CHANGED / NO_ELIGIBLE_WINDOW，探针拒绝继续。

这不是 PASS，也不作为兼容性失败结论；当前状态记为 BLOCKED。需要专用测试账号/会话和人工标注后继续。

## 未执行

- 真实 MessageSnapshot → Runtime::ingest
- AI / ReplyProvider 调用
- 微信输入、点击、回填、发送
- QQ 运行中验收
- 长稳、多 Space、多显示器、macOS 26
