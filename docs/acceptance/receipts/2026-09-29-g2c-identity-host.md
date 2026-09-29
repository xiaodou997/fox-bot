# G2c 会话身份、Ground Truth 与 Rust Host 本地验收回执

- 日期：2026-09-29
- 被测代码提交：`38c9ac665361c5956fc2ce7035ce4de1f88708fe`
- 分支：`feat/g2c-identity-host`
- 环境：macOS 27.0 / arm64，微信 4.1.13
- 结论：**G2c 实现与真实只读宿主链路完成；真实 MessageSnapshot→Observation 仍按设计保持 PROVISIONAL，等待 G2d 的专用测试会话 ground-truth 与显式 Binding。**

## 1. 固定提交集成门禁

固定 SHA 执行：

~~~bash
python3 scripts/g1_integration_check.py --with-macos-probe
~~~

结果：

- Rust：118 项通过
- cipher-disabled：1 项通过
- Python：49 项通过
- Swift：71 项通过
- format / Clippy / build-tools / HTTP smoke / host smoke / docs / whitespace：通过
- source fingerprint 前后均为 `361b43fc49249d1e7ec3bb2362210ed264407ce593537041cd0fdf7a2f2f95e4`
- external model requests：0
- native chat write/send operations：0

## 2. 真实微信 Rust Host → Worker → PrivateSnapshot

固定提交上，仅将已运行微信切到前台后执行只读探针。公开输出为：

~~~text
status                         SNAPSHOT_RECEIVED
application_session_state      STABLE_TWO_READS
conversation_fingerprint_state STABLE_TWO_READS
message_count                  12
partial_reasons                HEURISTIC_REGION, UNKNOWN_DIRECTION
worker.starts                  1
worker.successful_snapshots    2
worker.failures                0
raw_text_included              false
image_saved                    false
write_or_send_operations       0
observation_bridge             PROVISIONAL_REQUIRES_CONFIGURED_IDENTITY_AND_ACCEPTED_GROUND_TRUTH
~~~

该结果证明 Rust host 已接管 worker 生命周期，并能连续两次读取同一应用运行会话与会话视觉指纹；它**不证明当前 12 条消息方向、sender 或分组已经人工核对正确**。本次画面出现 `UNKNOWN_DIRECTION`，因此 fail-closed 门禁没有生成可路由 Observation。

## 3. application-session fallback

真实环境中 `NSRunningApplication.launchDate` 可为空。固定提交新增只读 fallback：使用 `proc_pidinfo(PROC_PIDTBSDINFO)` 读取目标进程启动时间，再与 bundle id 本地哈希生成 application-session fingerprint。

该 fingerprint：

- 不输出 PID、启动时间或哈希值；
- 同一进程会话两次读取必须一致；
- 进程重启后会变化，使旧 Binding/跨帧轨迹失效；
- 同一进程内登出/换号仍需显式 invalidate/rebind，当前不宣称自动检测。

## 4. Identity / Observation 门禁

真实 Observation 必须同时满足：

- 显式稳定 `Binding.key.account_binding / app_instance / conversation`；
- matching `identity_epoch`；
- matching application-session fingerprint；
- 唯一配置的 conversation fingerprint；
- 已接受且 strategy 匹配的 ground-truth 结果；
- 消息方向非 UNKNOWN、正文完整；
- 群聊 incoming 有 sender fingerprint；
- 至少两条跨帧 suffix/prefix 连续性。

首帧只做 historical baseline。无重叠、单条公共文本重叠、应用会话变化或未知方向均不会生成新工作。canonical id 不基于正文哈希，因此连续两条相同“好的”仍可识别为两条不同消息。

## 5. Ground-truth 框架

框架要求私有 fixture 位于 `target/g2c-groundtruth/`，输出仅包含脱敏计数。接受门槛：

- ≥6 case / ≥24 条消息；
- 覆盖 private、group、duplicate_text、numeric、multiline、reference；
- direction / sender / message-count error 为 0；
- 文本误差 ≤2%。

本阶段只用合成 fixture 验证框架；**没有采集或提交真实微信正文 ground-truth**。因此真实桥接保持 PROVISIONAL 是预期行为。

## 6. 下一阶段

G2c 到此收口。下一项 **G2d：真实 Observation Bridge 验收**：使用专用测试账号/会话建立私有人工标注和显式 Binding，验证真实新增消息从 `PrivateMessageSnapshot` 进入核心 `Runtime::ingest`。仍然不调用 AI、不写入、不发送。
