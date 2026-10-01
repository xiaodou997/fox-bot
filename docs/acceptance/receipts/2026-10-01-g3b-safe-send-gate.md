# G3b Safe Send Gate 回执

- 日期：2026-10-01
- 分支：`feat/g3b-safe-send-gate`
- 基线：`main@3fcfbbc0a68dbbc77b0384f70a6795ee0479e70b`
- 环境：macOS 27.0 / arm64；微信 4.1.13
- 结论：**Core / Host Gate PASS；MC-WX native no-send facts PASS，但 send-ready 因 COMPOSING_UNVERIFIED BLOCKED。**

## 1. 合并前置

此前 `feat/g2d-real-acceptance` 比 main 多 40 个提交，main 与 origin/main 均位于其祖先提交且没有分叉。

本地 fast-forward：

~~~text
main → 3fcfbbc
~~~

合并后完整门禁：

~~~text
Rust     131 PASS
Python    61 PASS
Swift     89 PASS
source_unchanged=true
~~~

旧 G2/G3a feature branch随后已安全删除；本回执不表示远端 main 已 push。

## 2. Core 双门禁

新增无副作用：

~~~text
preview_before_fill_gate
preview_before_send_gate
~~~

专项 Runtime 测试：47 / 47 PASS。

## 3. Host GUI ownership

Scheduler 边界：

- queued new incoming 先于 dispatch；
- queued manual own-output 取消旧回复；
- stop signal 阻止 side effect；
- DeviceOwner lock 被替换时在 fill/send 前失败。

Host 边界：4 / 4 PASS。

## 4. gate-only smoke

~~~json
{
  "status": "SAFE_SEND_GATE_PASS",
  "action_state": "PREPARED",
  "before_fill": {"allowed": true, "blockers": [], "phase": "BEFORE_FILL"},
  "before_send": {"allowed": true, "blockers": [], "phase": "BEFORE_SEND"},
  "fill_calls": 0,
  "send_calls": 0,
  "synthetic_outgoing_count": 0
}
~~~

## 5. MC-WX native no-send Gate

真实测试私聊最终结果：

~~~json
{
  "application_session_matches": true,
  "conversation_matches": true,
  "conversation_resolved": true,
  "draft_matches": true,
  "frontmost": true,
  "stable_two_reads": true,
  "recent_user_input": false,
  "ready": false,
  "blockers": ["COMPOSING_UNVERIFIED"],
  "write_operations": 0,
  "send_operations": 0,
  "network_requests": 0
}
~~~

公开输出不包含 draft 正文、conversation hash 或 app-session hash。

## 6. IME 证据探测

只聚焦输入区、不输入字符后：

~~~text
app AXFocusedUIElement          noValue
system-wide AXFocusedUIElement noValue
~~~

因此无法取得 selected-text / marked-text 语义状态。

Core 额外要求 `composition_verified=true` 才可 READY；不能用默认 `composing=false` 绕过。

## 7. 安全边界

本轮：

- real write：0；
- real send：0；
- Enter / Return：0；
- send-button click：0；
- external model request：0；
- screenshots saved：0。

G3c 继续 BLOCKED；不得用降低 Gate 条件换取 READY。

后续 IME Evidence Spike 没有找到 authoritative cross-process SAFE 信号；见 [2026-10-01-g3b-ime-evidence-spike.md](2026-10-01-g3b-ime-evidence-spike.md)。因此本回执的 BLOCKED 结论继续有效。
