# G3c-1 首次真实发送与回执修复

- 日期：2026-10-01
- 环境：macOS 27.0 / arm64 / 微信 4.1.13；无人值守独占执行契约。
- 真实发送使用代码：`61dfc96bd72214e49d32c22e0d26b386d18cd65a`；当时 checkout 为仅追加文档的 `2b128d2b44b78fe0afe606e22e64bcb076c531ff`。
- 修复代码：`a6baae5f223044a7ee83be062c65148aa0ecd389`。
- 结论：**首次真实回填/单次发送已执行，目标私聊出现匹配己方消息；原 RUN 的自动回执为 UNKNOWN。回执采集修复及离线回归通过，G3c-1 全自动闭环尚未最终验收。**
- 关联：[开发说明](../../development/G3C1_SINGLE_REAL_SEND.md) · [此前未发送回执](2026-10-01-g3c1-single-real-send.md)

## 1. 首次真实操作

用户明确“测试私聊已打开”后，先确认应用运行实例、会话绑定、前台、空输入区和发送按钮均符合。RUN `742961` 原先不存在 manifest、ledger 或回执。本轮创建了 FoxBot 专用 Keychain 账本密钥引用 `g3c1-test-20261001`，没有覆盖已有密钥；保留该 key 供此加密测试账本后续核对。

固定测试文字：`FoxBot G3c1 742961`。没有调用模型或真实 incoming 触发；测试触发来源仍为 SYNTHETIC_OPERATOR_TEST。

真实执行回执保存在本机 ignored 目录 `target/g2d-real/g2d-test/g3c-1/742961/first-execution-report.json`：

```json
{
  "run": "742961",
  "previous_state": "PREPARED",
  "action_state": "UNKNOWN",
  "encrypted_outbox": true,
  "native": {
    "inspect_requests": 3,
    "fill_requests": 1,
    "write_attempted": true,
    "send_requests": 1,
    "send_attempted": true,
    "reconcile_requests": 0,
    "last_status": "UNKNOWN"
  },
  "transition_history": ["PREPARED", "EXECUTING", "UNKNOWN"],
  "external_model_requests": 0
}
```

随后使用原版本、**相同 RUN**重放：fill_requests=0、send_requests=0、reconcile_requests=1，状态仍 UNKNOWN。这是真实的“未知状态不重发”验证，不是模拟测试。未创建新 RUN 或再次点击发送。

## 2. 只读确认消息存在与缺陷定位

原 G2 读取器在同一测试私聊识别到完整的 `FoxBot G3c1 742961`、方向 ME。独立只读几何诊断进一步确认：同一应用/会话，消息的顶部归一化坐标约 0.76674、底部约 0.77895；旧 SendPolicy 只保留 maxY < 0.765，因此漏掉了刚发出的底部消息。

同时，三条位于聊天中部的“今天/昨天 + 时间”分隔行进入旧回执消息数组，方向 UNKNOWN / complete=false。原匹配策略要求完整序列，因此也会阻止成功判定。旧哈希序列与后读序列未取得唯一连续重叠，不能仅删除不匹配项后宣称已完成自动核对。

以上只读观察证明当前界面存在匹配己方消息，不是平台送达、已读或完整自动回执通过的证明。所有公开诊断均未输出普通聊天正文，也未保存聊天截图。

## 3. 已完成的修复

发送回执采集边界改为复用现有 composer 的上边界（当前布局 y=0.79），保留其上方完整消息，拒绝跨边界文字。时间分隔行通过时间格式、居中位置、未知方向及高度联合识别；左右消息气泡中恰好出现相同日期文字时不删除。

提取 `receiptSignatures()` 用于可复现的纯策略测试。没有放宽发送前目标、输入内容、执行权或 UNKNOWN 不重发规则，也没有增加 IME/人工活动检测。

worker IPC 升级为 `foxbot.native-send-worker.v2`；新观察携带 `WECHAT_RECEIPT_V2`。旧回执缺少原始文字和坐标，无法在新规则下重新解析，不能反填 revision 或修改历史哈希。原 RUN 因此继续保留 UNKNOWN。版本隔离只是防止混用不同采集规则的证据，不是新的人工操作或 IME 前置要求。

原始 `receipt-context.json` SHA-256 在诊断前后保持：`0af9379be53316f6af8ca728e747b2e1db6ba28580086103cd0f6c458faf8146`。

## 4. 修复版真实只读复查

2026-10-01T18:38:59.819378+08:00，修复版连续两次观察，记录于本机 `post-fix-readonly-report.json`：

| 项目 | 第一次 | 第二次 |
| --- | --- | --- |
| status | OBSERVED | OBSERVED |
| 应用/会话/窗口/layout 与原回执一致 | true | true |
| evidence_revision | WECHAT_RECEIPT_V2 | WECHAT_RECEIPT_V2 |
| 消息数 | 10 | 10 |
| 不完整消息数 | 0 | 0 |
| 完整匹配测试文字的 ME 消息数 | 1 | 1 |
| draft_state | NONEMPTY | NONEMPTY |
| 本次写入/发送操作 | 0 / 0 | 0 / 0 |

后续输入区非空仅如实记录，未读取输出其正文，未覆盖或清空。这两次观察验证了漏裁和时间分隔采集修复，**不是新版本从 PREPARED 开始的完整发送验收**。

随后尝试修复版宿主对原 UNKNOWN RUN 再次只读重放，外层 90 秒超时退出，未取得宿主完成回执；该项不标 PASS，也不猜测具体阻塞点。超时后复核无遗留 foxbot-host / foxbot-macos-send 进程，原回执哈希不变，last-report 保留此前 UNKNOWN。完整宿主重放的超时仍需后续收口，不能由上述 native 只读成功替代。

## 5. 固定修复提交的回归

命令：`python3 scripts/g1_integration_check.py --with-macos-probe`。

报告：`target/g1-integration/59665b25c7c9443aaed4c723423ccc34/report.json`。

- head/head_after：`a6baae5f223044a7ee83be062c65148aa0ecd389`。
- source_before/source_after：`f74643146ef072acb7b2f2cfd2a33908602bcfb6d37767bf48246329773cfeda`。
- source_unchanged=true；13/13 检查通过。
- Rust 149、Python 61、Swift 113 全通过；禁用加密专项 1 通过；格式、Clippy、构建、模拟流程和文档检查通过。

新增测试覆盖底部消息、跨输入区边界排除、居中时间过滤、气泡内时间保留、不完整 OCR 不冒充完整证据、旧/未知 revision 不混用，以及旧回执不重写且不重发。

修复版产物 SHA-256：

| 产物 | SHA-256 |
| --- | --- |
| foxbot-host | `4cb6e27239716f1b1df4e982506e8e8984ae536d6911d0dc9f44168addde2587` |
| foxbot-macos-send | `5df6207d71a4f3c997e2fc231d61c66d665345dd9373e0cd4b50cc4902b1daad` |

## 6. 当前范围与下一项

单次回填和发送已取得真实操作记录，消息可见，原版本同 RUN 重放没有再次发送。尚未取得修复版从发送前新基线开始，自动进入 VERIFIED_OUTGOING 的完整回执；新版宿主重放另有一次超时未收口。因此 G3c-1 是 PARTIAL，不 Freeze，不宣称完整 AUTO_REPLY。

后续先定位宿主只读重放超时，再用修复版建立新的发送前基线并执行另一次明确授权的独立单条验收。原 `742961` 不重发，不通过删除账本、替换回执或换 RUN 假装重试。完成该增量后再接真实 incoming / ReplyProvider；多行长文、复杂表情、群聊、多会话及长稳仍未验收。
