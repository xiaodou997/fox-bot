# G3a Draft Writer / macOS 微信真机回执

- 日期：2026-10-01
- 分支：`feat/g2d-real-acceptance`
- 前置 G2 Freeze：`fe874c062e6377afffd0b355675b7dc264e5e230`
- 环境：macOS 27.0 / arm64；微信 4.1.13
- 结论：**测试级 draft fill + OCR readback REAL PASS；C05 仍为 HEURISTIC；SEND 未实现。**

## 1. 实现范围

新增：

~~~text
foxbot-macos-draft
WeChatDraftPolicy
shared NativeWindowSource
~~~

写入器与只读 OCR worker 为不同 executable target。Draft Writer 只接受显式测试参数 `--allow-heuristic-empty-test`；正文从 stdin 输入，公开 report 不包含正文或 conversation fingerprint。

## 2. AX 探测

真实 focused window：

~~~text
visited AX nodes     5
AXTextArea           0
AXTextField          0
settable AXValue     0
focused UI editor    unavailable
~~~

因此本轮没有宣称 AX 语义草稿读写可用。

## 3. 底层写入 smoke

首次低层 smoke 注入：

~~~text
FoxBot G3a Draft 731
~~~

整窗 Vision OCR 在输入区观察到该文本，证明 CGEvent Unicode 写入确实进入微信草稿，而不是只返回 API success。

该 smoke 从未触发 Enter 或发送。

## 4. 已有草稿保护

正式 `foxbot-macos-draft` 在上述草稿仍存在时运行另一段测试文本：

~~~json
{
  "draft_state_before": "NONEMPTY",
  "status": "DRAFT_NOT_EMPTY_OR_UNREADABLE",
  "write_attempted": false,
  "write_verified": false,
  "send_attempted": false
}
~~~

证明已有可见草稿不会被正式写入器覆盖。

## 5. 空草稿写入与回读

清除 FoxBot 自己产生的测试草稿后，输入框出现微信空状态占位：

~~~text
按住鼠标 语音输入文字
~~~

该字符串被 DraftPolicy 明确识别为空输入 UI 占位，不作为草稿正文。

正式写入：

~~~text
FoxBot G3a Formal 842
~~~

公开结果：

~~~json
{
  "draft_state_before": "EMPTY_HEURISTIC",
  "status": "DRAFT_WRITE_VERIFIED",
  "write_attempted": true,
  "write_verified": true,
  "send_attempted": false,
  "raw_text_included": false,
  "image_saved": false,
  "network_requests": 0
}
~~~

写后重新捕获同一 focused window，并重新校验 application-session / exact conversation fingerprint。OCR 回读与 expected 一致后才得到 `DRAFT_WRITE_VERIFIED`。

## 6. 安全边界

- 微信必须是系统 frontmost application；
- 写入前要求 exact conversation fingerprint；
- unresolved identity 不允许写；
- 已有可见草稿 / OCR unreadable 均拒绝；
- 不使用剪贴板；
- 没有 Return / Enter；
- 没有发送按钮 click；
- report 固定 `send_attempted=false`；
- 没有 ReplyProvider / HTTP 请求；
- 没有把 fill 成功升级成 send/receipt 成功。

## 7. 未完成

C05 不能完整验收，因为当前微信 AX 不暴露编辑器，OCR heuristic 尚不能可靠判断：

- IME 组字；
- Vision 漏识别的草稿；
- 用户与机器人并发编辑；
- 所有窗口布局 / 主题 / 字体变化。

因此 G3a 的 REAL PASS 只针对**显式测试级回填**。AUTO_REPLY / send 继续关闭，下一阶段为 G3b Safe Send Gate。
