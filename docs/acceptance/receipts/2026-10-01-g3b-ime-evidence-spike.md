# G3b IME Evidence Spike 回执

- 日期：2026-10-01
- 分支：`feat/g3b-ime-evidence-spike`
- 基线：`eeee63f49c52507e75eb9e0d51275984c952c2f9`
- 环境：macOS 27.0 / arm64；微信 4.1.13；Apple 简体中文输入方式
- 结论：**Spike COMPLETE；没有 authoritative cross-process composition-safe signal；G3c 继续 BLOCKED。**

## 1. 新增工具

~~~text
foxbot-macos-ime-evidence
NativeIMEEvidenceProbe
IMEEvidencePolicy
~~~

工具为 read-only：

~~~text
write_operations  0
send_operations   0
network_requests  0
image_saved       false
raw_text_included false
~~~

## 2. 微信前台基线

~~~json
{
  "authoritative_state": "UNAVAILABLE",
  "composition_verified": false,
  "composing": false,
  "target_frontmost": true,
  "app_focused_ui_available": false,
  "system_focused_ui_available": false,
  "selected_text_range_available": false,
  "input_source_available": true,
  "input_method_process_count": 2,
  "input_method_on_screen_window_count": 0,
  "recent_user_input": false,
  "positive_blockers": []
}
~~~

没有 blocker 不等于 SAFE，因为 authoritative state 仍 UNAVAILABLE。

## 3. System-wide AX 串应用修复

第一次未绑定 PID 的 system-wide probe 曾观察到 focused UI / selected range，但实际可能来自其它前台应用。

修复后：

- system focused element 必须能取得 PID；
- PID 必须等于当前微信根进程；
- 否则不计入微信证据。

微信前台重新验证后 target-scoped system focused UI 为 unavailable。

## 4. 输入源 / 输入法窗口

当前 input source 可以通过 TIS 取得，对应输入法 extension 正在运行。

未组字基线：

~~~text
input-method windows total     30+
input-method windows on-screen 0
~~~

输入法窗口 visibility 被接入 send-gate 为 positive blocker；0 个 on-screen 不会设置 composition_verified。

## 5. 受控 NSTextView 实验

为了避免直接修改微信，使用临时本地 `NSTextView`：

### CGEvent

事件没有进入临时文本 client，不能形成样本。

### NSTextView.keyDown

~~~text
input: nihao
hasMarkedText=false
IME on-screen windows=0
text=nihao
~~~

### NSTextInputContext.handleEvent

测试 context 的 selected input source 明确等于当前 Apple 简体拼音 source；5 个键盘事件均返回 handled=true，但仍：

~~~text
hasMarkedText=false
markedRange={5,0}
IME on-screen windows=0
text=nihao
~~~

因此受控实验没有成功构造真实 marked-text；不能据此把 candidate-window correlation 标为 verified。

## 6. Evidence 分类

~~~text
NSTextInputClient marked state  AUTHORITATIVE，但仅 receiver 自身可用
AX focused / selected range     CONTEXT_ONLY / 当前微信 unavailable
TIS current input source        CONTEXT_ONLY
recent input event              POSITIVE_BLOCKER
input-method visible window     POSITIVE_BLOCKER
absence of candidate window     NOT SAFE EVIDENCE
quiet time                      NOT SAFE EVIDENCE
~~~

## 7. Safe Send Gate 集成

共享 `NativeIMEEvidenceProbe` 已接入 `foxbot-macos-send-gate`。

当前不会产生：

~~~text
composition_verified=true
~~~

除非未来采集器真的获得 authoritative SAFE 状态。

## 8. 最终结论

本 Spike 不要求测试人员继续尝试输入法操作。

当前版本的 G3c Real Send 保持关闭；下一步应做支持策略选择或研究其它具备 authoritative composition semantics 的通道，而不是降低现有 Gate。
