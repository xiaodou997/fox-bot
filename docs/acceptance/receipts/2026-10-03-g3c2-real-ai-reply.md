# G3c-2 真实新消息、AI 回复与微信发送收口回执

- 日期：2026-10-03
- 最终收口代码提交：`a6495208cb0cecbe121dcebe7410cf50d49e363e`
- 初始真实模型请求与回填基线：`65c9439c1aedd27f3f6b764505a59c4ff4cdcd0c`
- 环境：macOS 27.0.1 / arm64 / 微信 4.1.13
- RUN：`ai-kimi-20261003-01`
- 接口选择：`kimi-coding`（API Key、请求正文和回复正文未进入本回执）
- 结论：**当前已绑定测试私聊的一条真实 incoming → 真实 AI 请求 → 原生回填 → 单次发送 → `VERIFIED_OUTGOING` 已通过；同 RUN 重放未再次请求模型或操作微信。**
- 关联：[G3c-2 开发说明](../../development/G3C2_REAL_REPLY.md) · [重绑定与 arm 回执](2026-10-03-g3c2-session-rebind-arm.md)

## 1. 实际端到端结果

测试对端在 arm 之后发送恰好一条短消息。本机保持原测试私聊前台、未切换会话、未人工编辑输入框。第一次 `g3c-reply-once` 实际完成：

```text
新 incoming 识别
→ 真实 HTTP ReplyProvider 请求 1 次
→ 模型返回一条 32 字符、单行、受支持范围内的回复
→ 原生回填 1 次
→ 回填 OCR 读回不确定
→ 未执行发送
```

首次公开结果为：

```json
{
  "action_state": "UNKNOWN",
  "model_jobs_this_invocation": 1,
  "native": {
    "read_requests": 2,
    "inspect_requests": 2,
    "fill_requests": 1,
    "send_requests": 0,
    "write_attempted": true,
    "send_attempted": null,
    "last_status": "DRAFT_MISMATCH"
  }
}
```

这不是模型或接口失败。账本确认只产生一个 service exchange；outbox 原因为 `fill_uncertain`，且发送前 receipt 尚不存在，因此可以确定 FoxBot 当时没有发起发送动作。没有换 RUN、删除账本、重新请求模型或重新回填。

## 2. 真机暴露的 OCR 光标问题

输入框中的实际回复为 32 个非 ASCII 字符；Apple Vision 在该帧把末尾插入光标额外识别成 ASCII `1`，公开诊断仅记录了长度、字符类别和摘要：期望 32 字符，观察 33 字符，唯一新增字符位于末尾。正文和摘要均未写入公开回执。

修复保持为**期望值感知的窄规则**：

- 期望回复必须非空、单行且全部为非 ASCII 字符；
- 观察值必须严格等于期望值加一个末尾 ASCII `1`；
- ASCII/混合文本、正文差异、其它数字位置或其它尾字符均不接受；
- 正常精确匹配与既有末尾 `|` 光标规则不变。

Swift 与 Rust 都执行相同边界检查。该规则不能把一般数字差异当作光标，也不修改模型回复正文。

## 3. 显式恢复，只发送已经存在的草稿

新增 `g3c-reply-recover-filled` 不是普通自动重试。它仅接受同一 RUN 中 `UNKNOWN / fill_uncertain` 的既有 action，并要求：

- 没有发送前 receipt，证明此前没有进入 send 请求；
- 当前账号会话、会话指纹、窗口和消息上下文仍与任务一致；
- arm 之后仍只有原来那一条新增 incoming，连续两读稳定；
- 当前输入框正文与账本中的完整 AI 回复严格对应；
- action 未超过 30 分钟恢复窗口；
- 不调用模型、不再次 fill，receipt 在点击发送之前落盘。

本次恢复结果：

```json
{
  "status": "VERIFIED_OUTGOING",
  "action_state": "VERIFIED_OUTGOING",
  "model_jobs_this_invocation": 0,
  "recovery": "EXISTING_FILLED_DRAFT_ONLY",
  "native": {
    "read_requests": 2,
    "inspect_requests": 1,
    "fill_requests": 0,
    "send_requests": 1,
    "write_attempted": null,
    "send_attempted": true,
    "last_status": "VERIFIED_OUTGOING"
  },
  "delivery_confirmed": false,
  "read_confirmed": false
}
```

账本状态序列为：

```text
PREPARED
→ EXECUTING / before_side_effect
→ UNKNOWN / fill_uncertain
→ EXECUTING / filled_recovery_before_send
→ VERIFIED_OUTGOING / matched_outgoing_not_delivery
```

发送后只读检查确认仍是已绑定测试私聊，输入框为空。`VERIFIED_OUTGOING` 只表示观察到对应的新己方消息，不表示微信平台送达或对方已读。

## 4. 同 RUN 重放不重复

恢复成功后再次执行原 `g3c-reply-once`，结果仍为 `VERIFIED_OUTGOING`，本次调用：

```text
model_jobs_this_invocation = 0
native.read_requests       = 0
native.inspect_requests    = 0
native.fill_requests       = 0
native.send_requests       = 0
```

因此同一 incoming 没有产生第二次模型请求、第二次回填或第二条微信消息。

## 5. 固定提交回归

在最终代码提交 `a6495208cb0cecbe121dcebe7410cf50d49e363e` 上执行：

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
```

最终报告：

```text
target/g1-integration/12eb13759b2442849c25caaee9086d6d/report.json
```

报告 `source_unchanged=true`、16/16 检查通过：

| 检查 | 结果 |
| --- | --- |
| Rust workspace / all targets | 184 PASS |
| 禁用加密专项 | 1 PASS |
| G3c 本地回复专项 | 16 PASS |
| 设置专项 | 10 PASS |
| Python | 61 PASS |
| Swift warnings-as-errors | 118 PASS |
| format / Clippy / build / bridge、gate、HTTP、host、settings smoke / docs / whitespace | PASS |

首次完整回归和随后一次 OCR 定向重跑中，macOS Vision 的三项真实合成图测试曾出现临时 `recognitionFailed`；再一次相同源码定向运行 14/14 通过，随后完整集成取得上述最终 PASS。没有修改这些 OCR 测试的通过条件，也不把失败隐去。

构建产物 SHA-256：

| 产物 | SHA-256 |
| --- | --- |
| `target/debug/foxbot-host` | `1f4250525cb09cc4633284025e371134f2f4759771a188add0cf150357dba268` |
| `target/macos-probe/debug/foxbot-macos-send` | `83b863594794988c8b35e1406026b5e4f5b8bd0dd3a1fcec28e235a0f0409cbe` |

## 6. 已通过范围与后续

本回执只接受：macOS 微信、已绑定当前测试私聊、单条新增文本、单条短回复、当前布局、一次真实 AI 调用与一次发送。回复仍限单行、最多 80 UTF-16 单元；长文、多行、复杂表情、批量消息、多会话导航和长期值守尚未由本次证明。

下一阶段可先扩展正常多行/较长回复，再进入当前私聊的持续值守与连续消息调度。无需重新讨论 IME 或人机同时编辑。
