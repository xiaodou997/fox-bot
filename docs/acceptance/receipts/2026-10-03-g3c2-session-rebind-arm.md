# G3c-2 微信重启后的会话重绑定与联调基线回执

- 日期：2026-10-03
- 被测代码提交：`0079c9a72e48146ca889b18e8cde657a2d254882`
- 环境：macOS 27.0.1 / arm64 / 微信 4.1.13
- 结论：**受控 application-session 重绑定 PASS；真实私聊只读 PASS；G3c-2 新 RUN 已 ARMED；真实模型请求与微信自动回复仍为 NOT_RUN。**
- 关联：[G3c-2 开发说明](../../development/G3C2_REAL_REPLY.md) · [多接口本地设置](../../development/LOCAL_SETTINGS.md)

## 1. 背景与边界

用户确认仍为此前测试使用的同一微信账号和同一测试私聊，并清空输入框。微信进程自旧验收后已重新启动，因此 application-session fingerprint 发生变化；conversation fingerprint 仍与已验收测试会话一致。旧 G2 baseline 与 verified 证据没有被改写。

新增 `g3c-rebind-session` 只刷新易失的 application-session fingerprint。它使用无写能力的原生 worker 连续读取两次，要求：窗口和消息上下文稳定、会话指纹不变、输入框为空、消息签名完整，并与此前已验收上下文至少存在两条连续重叠。重绑定记录单独保存到私有 G3c 文件，不改变 durable ConversationKey。

## 2. 真实重绑定结果

命令使用 `--confirm-same-account` 显式确认，本次输出为：

```json
{
  "status": "APPLICATION_SESSION_REBOUND",
  "stable_two_reads": true,
  "same_conversation": true,
  "continuity_overlap": 7,
  "message_count": 9,
  "draft_empty": true,
  "model_requests": 0,
  "write_operations": 0,
  "send_operations": 0,
  "raw_text_included": false
}
```

重绑定后再次执行私有读取检查，结果为 `PRIVATE_READ_VALIDATED`：当前帧 9 条消息、其中 5 条 incoming，输入框为空；模型、写入和发送次数均为 0。

## 3. G3c-2 基线已建立

正式本地配置已通过格式预检，选定接口 ID 为 `kimi-coding`。公开检查没有输出 endpoint 凭据或 API Key。

新的 RUN：

```text
ai-kimi-20261003-01
```

arm 结果：

```json
{
  "status": "ARMED_WAITING_FOR_NEW_MESSAGE",
  "connection_id": "kimi-coding",
  "model_jobs_this_invocation": 0,
  "encrypted_outbox": false,
  "native": {
    "read_requests": 2,
    "fill_requests": 0,
    "send_requests": 0,
    "reconcile_requests": 0
  }
}
```

该 RUN 使用本地普通 SQLite，已固定本次准备时的接口选择。arm 只记录当前历史基线；尚未调用真实模型、回填输入框或发送微信消息。后续只允许使用同一 RUN 处理一条新 incoming，不通过换 RUN 绕过 UNKNOWN 或重复任务。

## 4. 回归结果

固定源码执行 `python3 scripts/g1_integration_check.py --with-macos-probe`，最终报告：

```text
target/g1-integration/dd696bf6cfca430480d4f1f1ed9e9e89/report.json
```

最终 16/16 检查通过，`source_unchanged=true`：Rust 180、禁用加密专项 1、local reply 16、settings 10、Python 61、Swift 117；格式、Clippy、构建、HTTP/host/settings smoke、文档与 whitespace 全部通过。

此前一次完整回归中，macOS Vision 的三项真实 OCR 合成图测试出现临时 `recognitionFailed`；同一源码定向重跑 14/14 通过，随后再次执行完整集成取得上述最终 PASS。该瞬时失败不隐去，也未通过修改 OCR 条件规避。

构建产物 SHA-256：

| 产物 | SHA-256 |
| --- | --- |
| `target/debug/foxbot-host` | `57e5dcf87903c6e0b8737e86064feb5b18bea86256223869f47bd7dfc0453e53` |
| `target/macos-probe/debug/foxbot-macos-send` | `4c027bbff31081595b116f6e5f76f982da66347e2110d753904e12069250e60a` |

## 5. 下一项

由测试对端向当前私聊发送恰好一条新的短文本消息。本机不输入、不切换会话。随后使用同一 RUN `ai-kimi-20261003-01` 执行 once，验证真实 incoming → Kimi HTTP 回复 → 原生回填/发送 → `VERIFIED_OUTGOING`，并重放同 RUN 验证不重复调用模型或发送。在该步骤完成前，G3c-2 真实端到端仍是 NOT_RUN。
