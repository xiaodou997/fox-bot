# G3c-1 当前私聊单条发送回执

> 历史回执：记录首次开发时测试会话未就绪、尚未发送的结果。用户随后打开测试私聊，已执行一次真实发送；最新操作、回执修复与尚未完成项见[后续回执](2026-10-01-g3c1-real-attempt-and-receipt-fix.md)。下列旧结果保留，不代表当前仍未发送。

- 日期：2026-10-01
- 代码提交：`61dfc96bd72214e49d32c22e0d26b386d18cd65a`
- 分支：`feat/g3c1-single-real-send`
- 环境：macOS 27.0 / arm64 / 微信 4.1.13
- 最终状态：**IMPLEMENTATION + OFFLINE REGRESSION PASS；REAL SEND BLOCKED / NOT EXECUTED**
- 关联：[开发说明](../../development/G3C1_SINGLE_REAL_SEND.md) · [无人值守契约](../../development/G3B_UNATTENDED_EXECUTION.md)

## 1. 已完成的实现

原生单次回填/发送 worker、Runtime MessageChannel 适配、加密 outbox、发送前回执锚点、Swift/Rust 新己方消息双重核对以及重启后只读 reconcile 已实现。测试 CLI 使用固定文本 `FoxBot G3c1 742961` 与明确标注的 synthetic operator trigger；本轮没有接真实模型或持续 AUTO_REPLY。

## 2. 固定提交完整集成回归

命令：`python3 scripts/g1_integration_check.py --with-macos-probe`

本机完整报告：`target/g1-integration/f758f80d573d4c31bada75d0b4f816fa/report.json`。

- `head` / `head_after`：上述代码提交。
- `source_before` / `source_after`：`98c20b2c4856dfb2483561f61ad16ec442ee2027cc5d3846cdf3ecd9090ab0dc`。
- `source_unchanged=true`，13/13 检查通过。

| 检查 | 实际结果 |
| --- | --- |
| Rustfmt / Clippy `-D warnings` | PASS |
| Rust workspace / all targets | 147 PASS，0 failed |
| 禁用加密时拒绝明文降级专项 | 1 PASS |
| Rust 工具构建 | PASS |
| G2d bridge / G3b gate-only smoke | PASS，synthetic only |
| HTTP / Host smoke | PASS，本机测试服务，无外部模型 |
| Python tests | 61 PASS |
| Markdown / JSON / 案例引用 / whitespace | PASS |
| Swift warnings-as-errors tests | 107 PASS，0 failed |

新跨进程测试通过：单次执行、UNKNOWN 重启只读核对、不符回填不发送、缺少新消息的假成功被拒绝、错误请求 ID/只读副作用拒绝、超时回收、不盲目重启，以及绑定/执行锁保护。绑定专项确认原 G2 验证过的标题指纹变化可以衔接，应用重启或无关历史不能换绑。

首轮集成未通过的记录保留在 `target/g1-integration/bf2f87bd1dd6407f81ed7f0d9414eaa4/report.json`：并行 Python worker 冷启动使超时测试在 warmup 的 3 秒预算内未完成。修正仅分离测试启动预算（20 秒）和被测请求超时（仍 100 毫秒），仍断言进程在 3 秒内被回收；没有放宽真实发送或结果验证。

## 3. 原生只读观察与真实发送阻塞

早期只读观察曾确认当前会话与 G2 `verified-snapshot.json` 相符，输入框为空、发送按钮可定位，与验证记录有 8 条消息签名重叠。但旧 bridge-config 使用较早的标题 OCR 指纹；代码现已复用 G2 的 baseline→verified 连续性验证，保留原 durable key，不重新选择目标。

固定提交最终检查时，微信前台会话已不同于原测试私聊：

```json
{
  "status": "OBSERVED",
  "app_matches_verified": true,
  "conversation_matches_verified": false,
  "verified_overlap_count": 0,
  "suffix_prefix_overlaps": [],
  "message_count": 7,
  "draft_state": "EMPTY_HEURISTIC",
  "draft_characters": 0,
  "send_button_located": true,
  "write_attempted": false,
  "send_attempted": false
}
```

Host 因此报告 `CONVERSATION_MISMATCH`。这不是 IME 或人工活动门禁；当前窗口不是已授权的测试目标，不能向另一个会话发送。没有按当前窗口更改授权，也没有为了得到 PASS 删除身份检查。

本轮实际执行了只读采样和将微信置前台；**没有调用 g3c-send-once，没有回填新测试文本，没有点击发送，没有创建新测试 Keychain key或发送账本**。因此真实 C07/C08 与同 RUN 的真实重放验收仍为 NOT_RUN。不能将离线跨进程模拟发送计入真实发送次数。

## 4. 构建身份

| 产物 | SHA-256 |
| --- | --- |
| `target/debug/foxbot-host` | `9d0636a4ddc78f2fb620c0d039168d0a0817bc60e0ee86eaf24e86ad63efc605` |
| `target/macos-probe/debug/foxbot-macos-send` | `6894093d3bab63d2fd3aeb365beae40b2d2e0f395d46728f99fc13d5cda73f4a` |

版本/摘要复核时间：2026-10-01T18:15:08+08:00。没有进行长稳测试；上面的集成结果不是数小时或隔夜运行证据。

## 5. 继续验收所需条件

切回此前完成 G2/G3a 测试的私聊，保持微信前台，输入框留空。无需重新发送 incoming、准备拼音候选状态或配置模型。随后按开发说明使用固定 RUN 进行一次发送；不确定结果只读核对，不换 RUN 盲目重发。

通过真实发送后应另补回执，记录实际 native write/send 次数、PREPARED→EXECUTING→VERIFIED_OUTGOING/UNKNOWN、同 RUN 重放以及未覆盖范围。单行 80 UTF-16 单元、当前布局、私聊是本增量边界；多行/长文/复杂表情、失败图标、真实模型联调与多会话值守不在本轮完成声明内。
