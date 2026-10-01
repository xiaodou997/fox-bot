# G3b：IME Evidence Spike

> **已归档 / 不再阻塞 G3c（2026-10-01）**：用户明确实际运行无人值守、独占聊天界面。以下为旧人机共用假设下的研究记录；其中“必须证明 IME SAFE 才能开发发送”的要求已被[无人值守契约](G3B_UNATTENDED_EXECUTION.md)取代。实验事实保留，当前 send-gate 不再调用该 probe。

- 日期：2026-10-01
- 分支：`feat/g3b-ime-evidence-spike`
- 状态：**SPIKE COMPLETE / NO AUTHORITATIVE CROSS-PROCESS SAFE SIGNAL**
- 前置：[G3b Safe Send Gate](G3B_SAFE_SEND_GATE.md)
- 回执：[2026-10-01 IME Spike](../acceptance/receipts/2026-10-01-g3b-ime-evidence-spike.md)

## 1. 问题

G3b 的最后 blocker 是：

~~~text
COMPOSING_UNVERIFIED
~~~

问题不是“有没有中文输入法”，而是发送前能否 authoritative 地证明：

~~~text
当前目标微信输入框
AND
没有 marked text / conversion session
AND
prepared draft 是最终可发送文本
~~~

Spike 不允许通过等待、猜测、当前输入源名称或候选窗 absence 把 UNKNOWN 升级为 SAFE。

## 2. 证据分类

统一使用四类：

| 分类 | 用法 |
| --- | --- |
| AUTHORITATIVE | 可以直接决定 composition_verified / composing。 |
| POSITIVE_BLOCKER | 信号出现时可以阻断；信号消失不能证明 SAFE。 |
| CONTEXT_ONLY | 只能描述环境，不参与 READY 证明。 |
| UNAVAILABLE | 当前目标版本/系统无法取得。 |

## 3. Apple 文本输入语义

`NSTextInputClient.hasMarkedText()` 和 `markedRange()` 是文本输入 client 自己实现的 receiver 方法。对于 FoxBot 自己控制的文本视图，它们可以 authoritative 地说明 marked text；但 Accessibility / TIS 没有公开接口把另一个进程中的微信文本 client 对象暴露出来。

`AXSelectedTextRange` 只表示可编辑 Accessibility 元素中的选区，不等价于 marked range；而当前微信甚至没有暴露对应 focused editable AX element。

## 4. 新增共享 probe

新增：

~~~text
NativeIMEEvidenceProbe
foxbot-macos-ime-evidence
IMEEvidencePolicy
~~~

公开报告不输出输入源 ID、窗口名称、用户文本或窗口坐标。

主要字段：

~~~text
authoritative_state
composition_verified
composing
target_frontmost
app_focused_ui_available
system_focused_ui_available
selected_text_range_available
input_source_available
input_method_process_count
input_method_window_count
input_method_on_screen_window_count
recent_user_input
positive_blockers
signal_classes
~~~

## 5. 微信前台真实基线

当前微信测试会话、微信为系统前台：

~~~text
authoritative_state              UNAVAILABLE
composition_verified            false
composing                       false
target_frontmost                true
app_focused_ui_available        false
system_focused_ui_available     false
selected_text_range_available   false
input_source_available          true
input_method_process_count      2
input_method_window_count       30
input_method_on_screen_count    0
recent_user_input               false
positive_blockers               []
write_operations                0
send_operations                 0
~~~

这里 `composing=false` 只表示“没有 authoritative composing=true 的证据”，由于 `composition_verified=false`，它绝不能通过 Gate。

## 6. TIS / 输入源

真实当前输入源可识别为 Apple 简体中文输入方式，运行中的输入法 extension 也可定位。

这能说明：

- 当前 keyboard input source 是什么；
- 对应输入法进程是否运行。

不能说明：

- 微信当前是否有 marked text；
- 当前 conversion session 是否结束。

因此分类为 CONTEXT_ONLY。

## 7. 输入法窗口

当前输入法进程维护多组候选窗口对象；未主动组字基线中它们全部 `onScreen=false`。

规则：

~~~text
onScreen input-method window > 0
→ POSITIVE_BLOCKER

onScreen == 0
→ 不证明 SAFE
~~~

send-gate 已接入这一规则。

Spike 尝试用临时 `NSTextView` + 当前简体拼音 input context 构造受控 marked text：

1. CGEvent 输入没有进入临时 client；
2. `NSTextView.keyDown` 直接插入 ASCII；
3. `NSTextInputContext.handleEvent` 即使 selected input source 明确为当前 SCIM，仍直接插入 `nihao`，没有产生 marked text / candidate window。

因此没有把“候选窗与 marked text 的正相关”升级成已验证事实。它保持 positive-blocker heuristic。

## 8. 最近输入事件

`CGEventSourceSecondsSinceLastEventType` 可以说明最近是否有 key/mouse/scroll 活动。

规则：

~~~text
recent input
→ 阻断

quiet >= threshold
→ 只能说明近期没有事件
→ 不能证明 marked text 已结束
~~~

因此分类为 POSITIVE_BLOCKER。

## 9. AX / system-wide AX

Spike 对 system-wide focused element 加了 PID 约束：只有 focused AX element 的 PID 确实属于微信，才算目标证据，避免把浏览器/终端的 selected range 串进微信 Gate。

微信前台真实结果：

~~~text
app AXFocusedUIElement          unavailable
system-wide target-focused UI  unavailable
selectedTextRange              unavailable
~~~

因此当前没有 AX composition-safe 路径。

## 10. 接入 Safe Send Gate

`foxbot-macos-send-gate` 现在直接消费 `NativeIMEEvidenceProbe`：

- authoritative SAFE → 未来可令 `composition_verified=true`；
- authoritative COMPOSING → blocker；
- authoritative UNAVAILABLE → `COMPOSING_UNVERIFIED`；
- input-method on-screen window → 额外 blocker；
- recent user input → blocker。

当前版本仍是：

~~~text
composition_verified=false
ready=false
~~~

## 11. 结论

Spike 已完成，但没有解除 G3b blocker。

当前安全结论：

> 对 macOS 微信 4.1.13，现有公开跨进程 API 不能 authoritative 地证明 target editor 的 marked-text 已结束。

因此：

- 不进入 G3c Real Send；
- 不把候选窗 absence、静默时间、输入源名称、OCR draft 稳定等组合包装成“足够安全”；
- 若后续目标版本暴露真实 editable AX / marked-text 语义，可重新开启验证；
- 也可以评估其它发送通道或将该版本自动发送明确列为不支持。
