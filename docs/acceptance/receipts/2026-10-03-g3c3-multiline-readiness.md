# G3c-3 多行与较长纯文本回复实现回执

- 日期：2026-10-03
- 被测代码提交：`65d34df822a3155315e12285a9e1fd129ff5c82c`
- 分支：`feat/g3c3-multiline-replies`
- 环境：macOS 27.0.1 / arm64 / 微信 4.1.13
- 结论：**IMPLEMENTATION + OFFLINE REGRESSION PASS；真实微信多行 AI 回复 NOT_RUN。**
- 关联：[G3c-3 开发说明](../../development/G3C3_MULTILINE_REPLY.md) · [G3c-2 真实短回复回执](2026-10-03-g3c2-real-ai-reply.md)

## 1. 固定实现范围

本提交将当前微信纯文本发送范围从短单行扩展为最多 512 个 UTF-16 单元、最多 12 行。LF 可用；CR、Tab、其它控制字符、首尾空白和末尾竖线仍在任何原生写入前拒绝。超限回复不截断、不自动改写，保持 `UNSUPPORTED_REPLY_NO_WRITE`。

短单行继续使用 Unicode 事件包。多行或超过 80 个 UTF-16 单元的正文使用受控剪贴板路径：有界保存原剪贴板、粘贴正文、复制输入框完整正文逐字符核对、恢复原剪贴板。快照限制为 8 MiB、32 个 item、每个 item 64 个类型；无法完整保存或恢复时失败关闭。

本提交没有加入公共 Enter 发送、IME 检测或人机同时编辑兼容逻辑。发送仍然点击当前截图中唯一识别的“发送”按钮。

## 2. 上下文和回执变化

输入区变高可能遮住聊天区顶部历史。本实现只允许“从顶部丢失旧消息”的变化，且剩余至少两条连续、非全同的上下文锚点；底部新增、删改、乱序或重复锚点歧义均停止发送。

原生 IPC 更新为 `foxbot.native-send-worker.v5`，当前回执为 `WECHAT_RECEIPT_V4`。V4 在 exact digest 和 continuity digest 外增加 content digest，仅用于多行/较长回复的发送后视觉回执，以容纳微信气泡自动换行。发送前已经通过完整复制回读确认原文；content digest 不能允许任何非空白字符变化。

历史 V3 回执继续按原精确规则解析和核对，不补造 content digest、不修改旧 receipt、不提升旧 UNKNOWN；未知 revision 拒绝。

## 3. 固定提交完整门禁

命令：

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
```

报告：

```text
target/g1-integration/d5333cca8fca44398c35ff70c1fffa85/report.json
```

结果为 `source_unchanged=true`、16/16 检查通过：

| 检查 | 实际结果 |
| --- | --- |
| Rust workspace / all targets | 188 PASS |
| 禁用加密拒绝降级专项 | 1 PASS |
| G3c 本地回复专项 | 17 PASS |
| 设置专项 | 10 PASS |
| Python | 61 PASS |
| Swift warnings-as-errors | 123 PASS |
| Format / Clippy / 构建 / bridge、gate、HTTP、host、settings smoke / 文档 / whitespace | PASS |

新增或更新的回归覆盖：300 字回复、两段多行回复、超长/超行/Tab/CR 拒绝、剪贴板类型恢复、顶部历史遮挡、发送后旧历史重新显露、视觉换行回执、非空白字符变化拒绝、多行既有草稿恢复、V3 兼容和未知 revision 拒绝。

## 4. 构建身份

| 产物 | SHA-256 |
| --- | --- |
| `target/debug/foxbot-host` | `54beee3996a58ba71da08232c126afa00cc2b61ace1ca6da2ae6f774eef4d031` |
| `target/macos-probe/debug/foxbot-macos-send` | `20782233442d5734fedf072c07fd3e1fd22b03ccde93d295a60765677e9fe140` |

版本和摘要复核时间：2026-10-03T14:17:27+08:00。

## 5. 本轮没有做什么

本轮固定提交回归全部使用测试 HTTP 服务、合成消息和独立 worker。没有调用用户配置的真实模型，没有读取或修改 API Key，没有回填微信输入框，没有点击发送，也没有创建新的真实 G3c RUN。

用户当前本地配置仍保留之前的短单行提示词；代码和默认模板已允许分段，但不会静默覆盖已保存配置。因此，真实验收前需要先在设置页把提示词改为明确要求两段或较长回复。

## 6. 下一项

在已授权测试私聊中使用新 RUN：保持输入框为空，建立基线，让测试对端发送恰好一条新消息，使用真实模型生成两段或超过 80 个 UTF-16 单元的回复。完成标准：模型请求 1 次、fill 1 次、send 1 次、`VERIFIED_OUTGOING`；同 RUN 重放模型和所有原生调用均为 0，发送后输入框为空，原剪贴板内容保持。

在该回执产生前，G3c-3 只能标记为 IMPLEMENTED / OFFLINE PASS，不能宣称真实多行支持已 ACCEPTED，也不能据此进入多会话或正式发布。
