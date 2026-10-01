# G3c-1：当前私聊的单条真实发送

- 日期：2026-10-01
- 前置：[G2 Freeze](../acceptance/receipts/2026-09-30-g2-freeze.md) · [无人值守执行契约](G3B_UNATTENDED_EXECUTION.md)
- 状态：REAL SINGLE ATTEMPT / PARTIAL；已发送一次并只读识别到消息，旧自动回执仍 UNKNOWN；回执采集修复通过回归，完整修复版验收及宿主只读重放超时待收口，见[最新回执](../acceptance/receipts/2026-10-01-g3c1-real-attempt-and-receipt-fix.md)。
- 范围：macOS 微信、已绑定的当前测试私聊、固定短文本、单条发送与只读结果核对。不是完整 AUTO_REPLY，也不是多会话值守。

## 1. 闭环

```text
已验收的 G2 测试会话绑定
→ 固定测试回复进入加密 Runtime / PREPARED
→ BEFORE_FILL / EXECUTING 落盘
→ NativeSendChannel / 单次 Unicode 回填
→ 同一 surface、消息上下文和正文回读
→ BEFORE_SEND / 回执锚点落盘
→ 新鲜截图中唯一“发送”按钮 → 单次点击
→ 新的匹配 ME 消息 → VERIFIED_OUTGOING
                         或 UNKNOWN → 只读核对
```

G3c-1 使用明确标注为 `SYNTHETIC_OPERATOR_TEST` 的测试触发，回复固定为 `FoxBot G3c1 <RUN>`。它不冒充真实 incoming，不调用外部模型，不改变已有持续宿主的 synthetic 模式。下一增量再接真实 incoming 与 ReplyProvider。

## 2. 组件与边界

`foxbot-macos-send` 是独立 Swift worker。`--worker` 仅可预热、观察和核对；`--worker --allow-single-send` 才可执行一次 fill、一次 send。发送必须属于该 worker 已成功回填的 action 和文本，不能只拿一段文本直接按 Enter。父进程消失或前台目标改变时停止。

原生动作复用 `WeChatComposerProbe`。发送按钮通过当前单窗口截图的控制区域 OCR 查找唯一的“发送”或“Send”；找不到或出现歧义即不点击，不用通用 Return 或旧坐标兜底。现阶段只有已经实测布局的受限支持，不能推广为所有窗口高度、主题和语言。

写入暂限 **单行、最多 80 个 UTF-16 单元**；不允许控制字符、首尾空白或末尾竖线。以最多 20 个 UTF-16 单元的 Unicode 包注入，保留字符边界。长文本、多行、复杂表情与附件后续单独扩展；不截断回复后继续发送。

Rust `NativeSendChannel` 实现现有 `MessageChannel`，复用 Runtime 的双门禁、持久化状态和 DeviceOwner。私有 IPC 有大小、schema、请求 ID、超时和副作用字段检查；异常时结束并回收 worker，不在失败后静默重启并重发。`write_attempted/send_attempted=null` 表示没有可信原生回执，不应解释成肯定没有发生动作。

## 3. 回执不是布尔值

当前采集 revision 为 `WECHAT_RECEIPT_V2`，IPC 为 `foxbot.native-send-worker.v2`。采集保留 composer 上边界之前的完整消息，排除居中时间分隔；不删除左右气泡内的日期文字。旧回执仅存哈希，缺少用于重新解析的原文/坐标，不能自动补 revision 与新证据混用；保留 UNKNOWN，而不是迁移为成功。

旧 G2 聊天 ROI 与输入区部分重叠；发送回执观察额外排除输入区和工具条，防止把刚回填的正文当成已发消息。历史文字只转为 digest、方向、完整性签名，用于本地核对，不写入公开输出。

Swift 和 Rust 均检查：同一应用会话/聊天/窗口/布局、发送后输入区为空、消息识别完整、旧序列后缀与新序列前缀有唯一连续重叠、新增部分恰有一个匹配文本的 ME 消息。已有同文气泡、对方同文消息、错误会话、无连续性、重复歧义或不完整 OCR 均不能变成成功。

`VERIFIED_OUTGOING` 只表示观察到对应的新己方输出，**不表示送达或已读**。失败图标识别、网络送达状态和长期持续核对不在本增量已完成范围；UNKNOWN 不盲目改通道或重发。

## 4. 测试会话与持久化

入口要求 G2 的 acceptance、baseline 和 `verified-snapshot.json`。若最终验收时标题 OCR 指纹发生变化，通过原有 NativeObservationBridge 重放 baseline → verified 的已验证连续性取得最新指纹，保持原 durable ConversationKey；不因当前窗口“看起来像”就换绑。应用重启、跨账号或无关历史不能沿用。

每个 RUN 对应 `target/g2d-real/<SESSION>/g3c-1/<RUN>/` 的独立测试任务。该目录为 0700；manifest、回执签名与报告为 0600，回复进入 SQLCipher Runtime，密钥来自既有系统凭据库。manifest 持久化在调用任何写入之前，回执锚点持久化在 send 请求之前；原始聊天正文和截图不落入回执。

重复运行**相同 RUN**：PREPARED 经新观察后才可执行；VERIFIED_OUTGOING 直接报告原状态，不启动 native worker；UNKNOWN 只使用无写能力的 worker 核对。有孤立 ledger 而没有 manifest 时停止，不创建新 action 掩盖中断。不要改 RUN 来绕过 UNKNOWN；另一个 RUN 是一项新的人工授权测试，不是失败重试。

## 5. 命令

先构建和离线回归：

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
```

只读确认当前 G2 测试会话：

```bash
target/debug/foxbot-host g3c-inspect \
  target/macos-probe/debug/foxbot-macos-send g2d-test --allow-native-read
```

为该测试账本显式创建一个命名 Keychain key，已有 key 不替换。这里的名称只是凭据引用，不是密钥：

```bash
target/debug/foxbot-host init-key g3c1-test-20261001 --confirm-keychain-write
```

仅在明确授权的测试私聊执行一条消息。下面 RUN `742961` 已实际执行，当前只用于读取旧 UNKNOWN，不再发送；新的独立验收必须重新授权并建立发送前基线，不能换 RUN 绕过旧状态：

```bash
target/debug/foxbot-host g3c-send-once \
  target/macos-probe/debug/foxbot-macos-send \
  g2d-test 742961 g3c1-test-20261001 --allow-single-test-send
```

开发期间先暂停/停止已有 GUI 执行者，再交还微信前台；不要求 IME、候选窗或近期键鼠活动证明。正常目标/内容/权限检查仍然有效。

## 6. 回归与下一项

新增跨进程测试覆盖：成功只发一次、UNKNOWN 重启后只读核对、回填不符不发、假成功无新消息不认可、错请求 ID、只读响应携带写副作用、超时回收、错误绑定和丢锁。Swift 策略回归覆盖新旧同文、序列滚动/歧义、错误方向/会话、不完整 OCR、控制字符和发送按钮消歧。

本期不测试人工并发编辑。真实测试、短文本范围、执行次数、固定代码 SHA 和未覆盖项记录在独立回执中；不得用本设计说明宣称真实发送已通过。
