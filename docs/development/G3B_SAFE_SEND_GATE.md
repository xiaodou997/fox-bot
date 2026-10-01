# G3b：Safe Send Gate

- 日期：2026-10-01
- 状态：**UNATTENDED EXCLUSIVE CONTRACT；G3c 真实发送待实现/验收**
- 前置：[G2 Freeze](../acceptance/receipts/2026-09-30-g2-freeze.md) · [G3a Draft Writer](G3A_DRAFT_WRITER.md)
- 回执：[2026-10-01 G3b](../acceptance/receipts/2026-10-01-g3b-safe-send-gate.md)

## 1. 目标

G3b 不负责发送，而是把“现在是否允许进入发送动作”变成独立、可测试、无副作用的事实门禁。

2026-10-01 按用户确认的[无人值守独占契约](G3B_UNATTENDED_EXECUTION.md)修订：不考虑人工同时操作聊天界面，IME 和人工活动不再作为前置证明。

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
- permitted；
- frontmost。

`LiveTarget` 不再包含 IME/人工活动字段。这里没有把 unknown 写成 SAFE，而是从产品契约中删除不适用的人机共编条件。

### BEFORE_FILL

至少检查 host pause、action state、session revision、identity/profile、attempt budget、prior EXECUTING/SUBMITTED/UNKNOWN、target、application-session、conversation-surface、window/editor、frontmost、conversation_changed、permission 与 draft empty。

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
- 捕获后重新检查微信仍是前台；不查询 IME、候选窗或键鼠静默时间；
- 无 screenshot 保存、无网络、无 write/send。

针对瞬时 focused-surface 读取失败，只允许最多 3 次、200ms 间隔的有界只读重试；仍无法稳定时继续 BLOCKED。

## 7. IME 结论的适用范围已撤销

旧版 Gate 曾在其它目标/内容事实通过时，仅因 `COMPOSING_UNVERIFIED` 阻断；该真实记录保留在[旧回执](../acceptance/receipts/2026-10-01-g3b-safe-send-gate.md)，不能重写成新版 PASS。

新规则按无人值守独占运行，不再要求跨进程输入法状态证明；独立 [IME Spike](G3B_IME_EVIDENCE_SPIKE.md) 归档，不在主路径调用，也不再影响一期自动回复支持范围。

native 报告升级为 `foxbot.macos-send-gate.v2`，声明 `execution_model=UNATTENDED_EXCLUSIVE` 和 `input_state_policy=NOT_REQUIRED`。旧版的 recent_user_input / composition_verified / composing_state / input_method_window_visible 字段移除。NOT_REQUIRED 不表示探测到 SAFE；单测 READY 也不表示已经真实发送。

## 8. 下一步

进入 G3c-1 当前会话单条发送闭环：把现有测试级 Draft Writer 接入 Runtime，回填后复核文本和目标，调用应用专属单次发送，再匹配新增己方消息生成回执。先用固定测试回复，后接真实模型；不混入多会话导航和新的 IME 研究。

本轮没有新增 Enter / Return、发送按钮 click、真实 send channel 或写入授权开关；真实 C07/C08 仍待实现和指定测试会话验收。
