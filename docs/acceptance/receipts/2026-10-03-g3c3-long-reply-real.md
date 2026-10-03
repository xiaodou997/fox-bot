# G3c-3 较长回复真机闭环回执

- 日期：2026-10-03
- 被测代码提交：`5ee35ae9850b4de4355d5d43935bf726ee5b9bf3`
- 环境：macOS 27.0.1 / arm64 / 微信 4.1.13
- 会话：已绑定测试私聊 `g2d-test`
- RUN：`g3c3-multiline-20261003-145959`
- 结果：**REAL PASS（较长单行纯文本）；显式 LF 多行仍 NOT_RUN**
- 关联：[G3c-3 说明](../../development/G3C3_MULTILINE_REPLY.md) · [G3c-2 真实 AI 回执](2026-10-03-g3c2-real-ai-reply.md)

## 1. 本次真实闭环

测试对端在完成 arm 之后发送一条新的文本消息。FoxBot 使用已配置的 Kimi 接口生成回复，并在原绑定私聊执行：

```text
新 incoming
→ 真实模型请求 1 次
→ 有界剪贴板粘贴与完整复制回读
→ 输入框回填 1 次
→ 原生发送动作 1 次
→ 只读回执核对
→ VERIFIED_OUTGOING
```

模型返回正文为 **139 个 UTF-16 单元、1 个逻辑行**。它超过原 80 单元短回复范围，实际走 G3c-3 的剪贴板路径；但模型没有产生显式 LF，因此本回执不能证明“真正包含换行符的消息”已真机通过。

输入框回填成功意味着本轮已完成临时剪贴板写入、微信粘贴、Cmd+A/Cmd+C 完整回读以及原剪贴板恢复。正文没有被截断或重新生成。公开报告、本文和仓库均不保存 API Key 或回复原文。

## 2. 首次发送前停止与受控恢复

首次执行完成一次真实模型请求和一次 fill，但旧控制区域没有识别到长输入区右下角的发送按钮：

```text
last_status    = SEND_BUTTON_UNAVAILABLE
fill_requests  = 1
send_requests  = 1
write_attempted = true
send_attempted  = false
```

因此当时**没有发生点击或发送**，action 记录为 `UNKNOWN / effect_unconfirmed`，完整草稿保留在输入框。没有更换 RUN、重新请求模型或重新填入正文。

修复将发送按钮 OCR 限定为更紧的右下角区域，并为 `send_attempted=false` 增加独立恢复语义。恢复前同时验证：

- 同一持久化 action、目标与回复摘要；
- 旧报告明确为 `SEND_BUTTON_UNAVAILABLE`；
- `send_attempted=false`，且发送前 receipt 中没有按钮坐标；
- 当前输入框复制回读仍与账本正文精确一致；
- 当前会话、窗口和 incoming 上下文仍连续；
- action 在 30 分钟恢复窗口内。

恢复只发送现有草稿，模型请求和 fill 均为 0。物理发送动作只发生一次。

## 3. 长气泡的视觉回执收口

消息发出后，微信将较长气泡拆成多个 OCR 片段，并出现少量视觉识别误差；同时触发消息的一项 continuity OCR 发生漂移。发送前正文已经通过剪贴板复制逐字符验证，因此后置回执采用受限的视觉容错：

- 仅适用于多行或超过 80 UTF-16 单元的正文；
- 非空白正文至少 64 字符；
- 首尾各 8 个字符必须精确锚定；
- 长度差不超过 2，编辑距离不超过 2；
- 最多合并 3 个相邻尾部 OCR 片段，首片必须为己方；
- 不允许 UNKNOWN 方向片段；
- 仅允许最后一个对方上下文锚点出现一次 OCR 漂移，且此前至少保留 5 个连续锚点；
- 原始 OCR 正文只在私有 worker IPC 中使用，公开结果继续脱敏。

原生 IPC 更新为 `foxbot.native-send-worker.v6`，回执仍为 `WECHAT_RECEIPT_V4`。短回复继续要求精确摘要；旧 V3 receipt 不补造新证据、不改写历史状态。

使用修复版进行只读 reconcile 后得到：

```text
action_state                 = VERIFIED_OUTGOING
status                       = VERIFIED_OUTGOING
model_jobs_this_invocation   = 0
fill_requests                = 0
send_requests                = 0
reconcile_requests           = 1
last_status                  = VERIFIED_OUTGOING
```

`VERIFIED_OUTGOING` 只表示在目标聊天中观察到对应的新己方输出，不代表微信送达或对方已读。

## 4. 持久化状态与零重放

最终账本状态：

```text
PREPARED / prepared
→ EXECUTING / before_side_effect
→ UNKNOWN / effect_unconfirmed
→ EXECUTING / unattempted_recovery_before_send
→ UNKNOWN / effect_unconfirmed
→ VERIFIED_OUTGOING / matched_outgoing_not_delivery
```

中间数次 `UNKNOWN` 来自发送后的只读回执核对，没有第二次模型请求、fill 或发送。最终同 RUN 再执行一次时：

```text
model_jobs = 0
read       = 0
inspect    = 0
fill       = 0
send       = 0
reconcile  = 0
```

账本仅有 1 个任务和 1 个模型 exchange，证明没有通过新 RUN 或重建任务掩盖未知副作用。

## 5. 固定提交回归

完整门禁命令：

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
```

最终报告：

```text
target/g1-integration/2f6b01a95eed4a2aaf32f23cea0fa67c/report.json
```

检查期间 `source_unchanged=true`，16/16 通过：

| 检查 | 结果 |
| --- | ---: |
| Rust workspace / all targets | 191 PASS |
| 禁用加密专项 | 1 PASS |
| G3c 本地回复专项 | 18 PASS |
| 设置专项 | 10 PASS |
| Python | 62 PASS |
| Swift | 124 PASS |
| Format、Clippy、构建、smoke、文档与 whitespace | PASS |

开发过程中曾观察到 Apple Vision 在模型预热后的首个请求瞬时返回 `recognitionFailed`，以及测试监管进程的 process-group kill 在受限环境返回 `PermissionError`。最终代码分别增加一次新的本地 Vision request 重试，以及对子进程直接 kill 的回退；没有降低 OCR 内容断言或发送门禁。

构建产物摘要：

| 产物 | SHA-256 |
| --- | --- |
| `target/debug/foxbot-host` | `22007125e960d028b76729324ee04b5dc55390eb787e163a8b90b15d367a6c6b` |
| `target/macos-probe/debug/foxbot-macos-send` | `04c85323ec601e29fec76bd6113b7c287c326520fcbb1e542cfc8c37623779e1` |

## 6. 已证明范围与下一项

本回执接受：macOS 微信、一个已绑定测试私聊、一条真实 incoming、一次真实 AI 请求、一条 139 UTF-16 的较长纯文本回复、一次真实发送、自动结果确认和同 RUN 零重放。

仍未由本次证明：显式 LF 多行消息、接近 512 单元上限、复杂表情、附件、连续多条 incoming、持续值守、多会话导航、平台送达和已读状态。下一步应先用固定包含 LF 的回复完成一条真机测试，再进入当前私聊的持续自动值守。
