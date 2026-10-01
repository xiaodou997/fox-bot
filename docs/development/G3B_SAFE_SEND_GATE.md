# G3b：Safe Send Gate

- 日期：2026-10-01
- 状态：**CORE / HOST PASS；MC-WX NATIVE SEND-READY BLOCKED**
- 前置：[G2 Freeze](../acceptance/receipts/2026-09-30-g2-freeze.md) · [G3a Draft Writer](G3A_DRAFT_WRITER.md)
- 回执：[2026-10-01 G3b](../acceptance/receipts/2026-10-01-g3b-safe-send-gate.md)

## 1. 目标

G3b 不负责发送，而是把“现在是否允许进入发送动作”变成独立、可测试、无副作用的事实门禁。

~~~text
PREPARED outbound
  ↓
BEFORE_FILL
  ↓
允许平台回填
  ↓
重新 inspect
  ↓
BEFORE_SEND
  ↓
只有 READY 才允许未来 G3c 调用 send
~~~

本阶段没有新增 Enter / Return、发送按钮 click 或真实 send channel。

## 2. Core Gate

`LiveTarget` 现在显式携带：

- stable ConversationKey / identity_epoch；
- application-session ref；
- conversation-surface ref；
- window / editor ref；
- layout revision；
- draft；
- conversation_changed；
- `composition_verified` 与 composing；
- user_active；
- permitted；
- frontmost。

`composing=false` 不再自动代表安全；只有 `composition_verified=true && composing=false` 才满足该项。

### BEFORE_FILL

至少检查 host pause、action state、session revision、identity/profile、attempt budget、prior EXECUTING/SUBMITTED/UNKNOWN、target、application-session、conversation-surface、window/editor、frontmost、conversation_changed、composition_verified/composing、user_active、permission 与 draft empty。

### BEFORE_SEND

fill 后重新检查全部 live safety 条件，并额外要求：

- application-session、conversation-surface、window、editor、layout 与写前 surface 完全一致；
- draft 必须与 prepared outbound text 完全一致。

任何 readback / surface / draft 不确定都不能调用 send。

## 3. Dispatch 复用同一 Gate

旧 `dispatch()` 不再维护另一套散落判断：

- persistent blockers 在任何平台 inspect 前检查；
- stale revision → STALE；
- 其它写前 blocker → BLOCKED；
- fill 后 Gate 失败 → UNKNOWN；
- 只有 BEFORE_SEND READY 才可能进入未来的 channel.send。

attempt budget 只在 PREPARED 跨入 EXECUTING 前消费一次，不能在同一次已获授权的 EXECUTING action 上二次拒绝。

## 4. Host 边界

Scheduler 在 dispatch 前继续保证：

- DeviceOwner 有效；
- input queue 先于 ready send drain；
- 新 incoming 使旧 revision stale；
- 新的己方人工输出取消旧自动回复；
- stop/control signal 优先；
- DeviceOwner lock inode 被替换时在 fill/send 前失败。

## 5. gate-only synthetic smoke

`foxbot-sim gate-only` 创建真实 PREPARED outbox action，但只做两次 preview；draft 的“已回填”状态只在内存 clone 中模拟。

固定 PASS：

~~~text
before_fill.allowed  = true
before_send.allowed  = true
action_state         = PREPARED
fill_calls           = 0
send_calls           = 0
synthetic_outgoing   = 0
~~~

## 6. macOS 微信 native no-send Gate

新增 `foxbot-macos-send-gate`，复用 `WeChatComposerProbe`，只做读取：

- 微信必须是系统 frontmost；
- focused AX frame ↔ ScreenCaptureKit window 唯一绑定；
- 同一 surface 连续读取两次；
- application-session exact match；
- conversation fingerprint resolved + exact match；
- expected draft 两次 OCR exact-match；
- 最近 1 秒没有 key/mouse/scroll 人工输入；
- 无 screenshot 保存、无网络、无 write/send。

针对瞬时 focused-surface 读取失败，只允许最多 3 次、200ms 间隔的有界只读重试；仍无法稳定时继续 BLOCKED。

## 7. 当前唯一真机 blocker：IME composing

微信 4.1.13 在本机：

- app AX root 没有 `AXFocusedUIElement`；
- system-wide AX 同样返回 focused UI element noValue；
- 无可读 `AXSelectedTextRange` / marked-text 语义节点。

因此不能把“没有看到 composing”写成 `composing=false`。

最终 no-send gate：

~~~text
application_session_matches  true
conversation_matches         true
conversation_resolved        true
draft_matches                true
frontmost                    true
stable_two_reads             true
recent_user_input            false
write_operations             0
send_operations              0
ready                        false
blockers                     COMPOSING_UNVERIFIED
~~~

这是预期 fail-closed 结果。

## 8. 下一步

G3c Real Send **不能开始**，直到有可信方式证明 composing/marked-text 已清空。

可继续研究：

1. 是否存在目标微信版本可用的系统文本输入状态证据；
2. 若无法取得，是否将自动发送支持限制到能够证明 composition-safe 的版本/输入通道；
3. 不能用“静默 1 秒”“当前输入源名称”或“候选窗没看到”单独替代 composing 事实。

任何方案都必须先在 gate-only 模式验证，再开放 send action。
