# G2d：PrivateMessageSnapshot → Observation → Runtime 桥接

- 日期：2026-09-29
- 状态：桥接实现与合成端到端验收已通过；真实微信 Observation 验收仍 BLOCKED，原因是没有 accepted 真实 ground-truth + 显式 Binding，且本轮后续真实窗口出现 TARGET_CHANGED / NO_ELIGIBLE_WINDOW。
- 前置：[G2c 会话身份与 Rust Host](G2C_IDENTITY_HOST.md)
- 关联：[设计基线](../design/BASELINE.md) · [能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [G2d 回执](../acceptance/receipts/2026-09-29-g2d-observation-bridge.md)

## 1. 两阶段桥接

`NativeObservationBridge::ingest_into_runtime` 不直接在生成 Observation 时前移跨帧游标，而是：

~~~text
clone bridge state
  ↓
bridge(snapshot)
  ↓
Runtime::ingest(each Observation)
  ↓ 全部成功
commit cloned bridge state
~~~

如果 Runtime 在中间一条发生 backpressure/storage error，bridge 原状态不前移。此前已经持久化的 prefix 在重试时使用相同 canonical id，被 Runtime 识别为 Duplicate；不会生成新 id，也不会丢掉未写后缀。

## 2. Runtime 结果映射

桥接只调用 `Runtime::ingest`，不调用 ReplyProvider、HTTP、MessageChannel::fill/send 或任何原生输入 API。结果按以下计数输出：

- BASELINE → historical Observation → Runtime Baseline
- NEW incoming → 可能 Queued
- own message → Ignored
- retry prefix → Duplicate
- provisional / ambiguous → 0 Runtime writes

应用 session、conversation fingerprint、ground-truth、方向/sender/完整度与两条连续性门禁继续沿用 G2c。

## 3. 背压与幂等重试

测试将 `max_pending/max_batch` 设为 1，并一次产生两条新增 incoming：第一条成功 Queued，第二条触发 Backpressure。随后暂停 Runtime 后重试相同快照：

~~~text
first new message   -> Duplicate
second new message  -> Ignored (host paused)
bridge cursor       -> only now committed
~~~

最终消息总数没有出现重复记录。这个测试验证的是恢复语义，不代表生产中应通过 pause 消化背压。

## 4. 开发 smoke

入口：

~~~bash
foxbot-host bridge-sim-probe STATE --allow-plaintext-synthetic
~~~

它创建一次性合成 Runtime，运行 baseline → one new incoming → repeat，并删除测试 state。它不会启动网络、模型或聊天原生适配器。

固定代码提交实际输出要点：

~~~text
baseline: BASELINE / 3 baselined
new:      NEW / 1 queued
repeat:   NO_CHANGE / 0 observations
runtime:  messages=4, tasks=0, ready=0, unresolved_sends=0
external_model_requests=0
native_chat_operations=0
write_or_send_operations=0
~~~

`g2d-bridge-smoke` 已加入 `scripts/g1_integration_check.py`。

## 5. 真实微信边界

G2d 没有伪造真实 ground-truth。G2c 固定提交曾证明真实 Rust Host 两帧 application-session / conversation fingerprint 稳定；G2d 固定提交后的后续真实读取过程中，目标窗口状态发生变化，worker 脱敏报告出现 TARGET_CHANGED / NO_ELIGIBLE_WINDOW，因此读取被拒绝。

这属于预期 fail-closed：没有放宽窗口匹配，也没有把上一帧或旧截图继续送进 Runtime。

## 6. 真实验收仍需要什么

要把 G2d 从 BLOCKED 提升为真实 PASS，需要在专用测试会话中：

1. 在 `target/g2c-groundtruth/` 建立人工 expected/observed 标注；
2. 通过 ground-truth 门槛；
3. 显式把当前微信 conversation fingerprint 绑定到稳定 ConversationKey；
4. 先读取 historical baseline；
5. 从另一测试账号发送一条已知新消息；
6. 验证只生成一个 incoming Observation，且 `Runtime::ingest` 为 Queued；
7. 重读同一画面必须 NO_CHANGE / Duplicate-safe；
8. 全程不调用 AI、不写入微信、不发送回复。

真实发送继续属于 G3。
