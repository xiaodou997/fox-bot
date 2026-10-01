# G3a：macOS 微信 Draft Writer

- 日期：2026-10-01
- 状态：**REAL PASS / TEST-ONLY DRAFT FILL**
- 前置：[G2 Freeze](../acceptance/receipts/2026-09-30-g2-freeze.md)
- 关联：[能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md) · [真实回执](../acceptance/receipts/2026-10-01-g3a-draft-writer.md)

## 1. 目标

G3a 只回答一个问题：

> FoxBot 能否在已经确认的当前微信测试私聊中，保护现有可见草稿、写入一段明确测试文本，并在不发送的前提下回读确认。

本阶段明确不做：

- Enter / Return；
- 点击发送按钮；
- 模型调用；
- OutboundAction 状态推进；
- 多会话导航；
- 剪贴板写入或恢复；
- 把 `fill` 成功当作 `send` 成功。

## 2. 为什么不使用 AXValue

真实微信 4.1.13 / macOS 27.0 下：

- focused window 的 AX 树仅暴露约 5 个节点；
- 没有可写的 AXTextArea / AXTextField；
- 没有 settable `AXValue`；
- 点击输入区后仍没有可用的 `AXFocusedUIElement`。

因此 G3a 不伪造“语义编辑器已找到”。当前实现使用：

~~~text
AX + ScreenCaptureKit
  → 唯一 focused window
  → Apple Vision 草稿区域 preflight
  → 鼠标只点击输入区安全点
  → CGEvent Unicode 文本注入
  → 同窗口再次截图
  → OCR 回读并与 expected 完全比较
~~~

## 3. 独立写入程序

新增产品：

~~~text
foxbot-macos-draft
~~~

调用形态：

~~~bash
printf 'FoxBot G3a Formal 842' | +  target/macos-probe/debug/foxbot-macos-draft +  --expected-conversation <64-char-fingerprint> +  --allow-heuristic-empty-test
~~~

草稿正文只从 stdin 进入，不放 argv；公开 stdout 只输出闭合状态，不输出正文、窗口标题或 fingerprint。

`--allow-heuristic-empty-test` 是明确的测试门。当前实现没有生产级“自动空草稿确认”，因此没有这个参数就不能写。

## 4. 写前门禁

所有条件必须同时满足：

1. 微信根实例唯一；
2. Accessibility / Screen Recording 已授权；
3. 系统 frontmost application 必须就是微信；
4. AX focused-window geometry 与 ScreenCaptureKit 窗口唯一匹配；
5. application-session 可计算；
6. conversation fingerprint 必须与命令传入 exact match；
7. conversation identity 不能是 unresolved；
8. 草稿 OCR 必须是 `EMPTY_HEURISTIC`；
9. 输入文本非空、UTF-8、≤4096 bytes。

任意一项失败都不注入文字。

## 5. 草稿状态

当前 C05 只能报告：

~~~text
EMPTY_HEURISTIC
NONEMPTY
UNREADABLE
~~~

`EMPTY_HEURISTIC` 的含义不是“已经语义读取到空字符串”，而是：

- 本地 Vision 完整识别成功；
- draft ROI 中没有正文；
- 微信已知空输入占位文案不计为草稿；
- 右侧发送/控制区域排除。

因此 C05 目前不能标记为完整 ACCEPTED。仍需验证文本存在但 Vision 漏识别，以及主题/字体/窗口布局使正文超出 ROI 的情况。

2026-10-01 用户明确实际使用为无人值守独占，人工并发输入和 IME 组字不在本期范围，不再阻塞 AUTO_REPLY 开发。具体变更见[无人值守执行说明](G3B_UNATTENDED_EXECUTION.md)。输入区残留和回读可靠性仍按实际通道验收。

## 6. 写入与回读

写入不使用剪贴板，避免覆盖用户当前 clipboard。

流程：

~~~text
before capture
  → exact identity
  → EMPTY_HEURISTIC
  → click composer safe point
  → re-check WeChat is frontmost
  → inject Unicode text
  → after capture
  → app-session unchanged
  → conversation fingerprint unchanged
  → OCR draft region
  → normalized text == expected
~~~

Vision 偶尔会把插入光标识别成末尾 `|`，回读规范化只允许移除这个末尾 caret 噪声与折叠水平空白，不做语义改写。

## 7. 真实门禁

### 7.1 已有草稿拒绝覆盖

输入框存在 FoxBot 测试草稿时运行正式 Draft Writer：

~~~text
draft_state_before  NONEMPTY
status              DRAFT_NOT_EMPTY_OR_UNREADABLE
write_attempted     false
write_verified      false
send_attempted      false
~~~

### 7.2 空草稿写入 / 回读

清空测试草稿后，输入框只显示微信空占位文案。运行：

~~~text
FoxBot G3a Formal 842
~~~

结果：

~~~text
draft_state_before  EMPTY_HEURISTIC
status              DRAFT_WRITE_VERIFIED
write_attempted     true
write_verified      true
send_attempted      false
~~~

整窗 OCR 同时确认草稿文字真实出现在微信输入区；没有发送气泡产生的动作证据。

## 8. 下一步

G3a 到此只冻结“测试级安全回填”。

下一阶段 G3b Safe Send Gate 必须至少加入：

- current app-session re-check；
- conversation / continuity re-check；
- incoming revision re-check；
- 显式暂停 / DeviceOwner 执行权；
- draft 是否仍与 prepared outbound 完全相等；
- pending / UNKNOWN outbound 检查；
- 发送动作前最后一次 GUI ownership。

G3b 已按[无人值守独占契约](G3B_UNATTENDED_EXECUTION.md)调整；下一步是 G3c 真实发送实现与验收，不再等待 IME 证明。本工具依然是测试级回填入口，本轮不更改其写入授权参数。
