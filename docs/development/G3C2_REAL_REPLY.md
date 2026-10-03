# G3c-2：当前私聊真实新消息与 AI 回复联调

- 日期：2026-10-01
- 基线：G3c-1 已在测试私聊完成固定短回复发送与自动确认。
- 本增量：单次联调入口 IMPLEMENTED / OFFLINE PASS，真实私聊只读检查通过；实际模型端到端待配置，见[本轮回执](../acceptance/receipts/2026-10-01-g3c2-real-reply-readiness.md)。不能继承离线 HTTP fixture 或 G3c-1 的真实发送 PASS。

## 1. 用户流程

```text
配置模型接口与凭据
→ 原测试私聊滚动到底部、输入框为空
→ arm：保存当前聊天基线，不调用模型，不发送
→ 对方发送一条新消息
→ once：识别相对基线新增的对方消息
→ HTTP ReplyProvider 返回完整回复
→ 重新核对当前会话与消息
→ 原生回填 / 单次发送 / 自动确认
```

arm 是测试入口的“开始接收新消息”，不是逐条人工批准，也不负责手动编辑。它可在一次命令完成后退出，用户发送新消息之后再运行 once，不必抢在后台计时结束前操作。once 也支持等待新消息，默认轮询 60 秒；实际总耗时另包括有界的原生读取、模型调用和反馈。30 分钟前建立的 arm 失效，不继续对旧视图自动回复。

只识别一个新增 incoming。已有历史、自己的消息、时间分隔、输入框内容都不冒充 incoming；一次出现多条新增消息或丢失连续性时停止，该阶段尚不合并批量消息。重复内容通过序列位置区分，不直接按文字哈希去重。实际运行沿用无人值守独占约定，不探测 IME/人工活动。

## 2. 实现与数据流

`g3c_reply.rs` 接现有 `HttpReplyService::begin/run/finish` 与 `Runtime::ingest/prepare_send/dispatch`。模型返回的是要发送的正文，不再替换成固定测试文本。普通 Chat Completions 与 BusinessV1 均复用原 HTTP 实现；自定义业务服务使用 `provider.kind=custom`，不会额外塞入系统提示词。业务服务的反馈走原有持久化 receipt outbox，与聊天发送分开。

`foxbot-macos-send` 增加只读 `read` 命令。一次截图中的正文、方向和完整性与回执签名来自同一解析投影；宿主复核原文 digest、上下文 digest 和方向，拒绝不对应的数据。原文仅通过该私有命令传给宿主；inspect、报告、公开日志仍只输出状态和计数。输入区、时间分隔的排除沿用 G3c-1 修复。

IPC 更新为 `foxbot.native-send-worker.v4`，**读取/回执解析规则仍为 WECHAT_RECEIPT_V3**，没有仅因增加命令就重写历史回执。需要一起构建 Rust host 和 Swift worker。G3c-1 的固定文本入口继续保留。

模型调用之前将 generation claim 持久化；中断后不会自动再调用一次模型。HTTP 本增量仅允许 max_attempts=1 / max_in_flight=1。收到模型结果后重新读取，若会话、窗口、上下文变化或读取失败，拒绝写入。dispatch 还被固定到生成该回复时的消息上下文，避免把第一次“模型后读取”误当成新的无条件基线。

每个 RUN 仅处理一次模型回复。已完成任务重放不再生成/回填/发送；UNKNOWN 且有发送前回执时仅只读核对；进程在生成中退出时返回 INTERRUPTED_NO_AUTOMATIC_RETRY。不会换 RUN 重试同一未知副作用，也不删除旧账本。此处是单条联调，不承诺多个不同 RUN 的跨任务全局去重或常驻调度。

## 3. 支持范围

**本次先接已有可靠写入器，正文仍限短单行、最多 80 UTF-16 单元。** 普通中文可以通过；长文、多行、复杂表情没有借这次“接模型”自动取得支持。超出范围返回 UNSUPPORTED_REPLY_NO_WRITE，已生成的完整回复保留在加密 Runtime 中，输入区写入/发送均为 0，不截断后发送。

模板的短回答 system_prompt 只属于普通模型的联调示例，不会强加给已配置的业务服务；模型不遵循字数或单行要求时也不能擅自缩短内容。正常长文本/多行支持作为后续独立增量。

模型可以返回 no_reply/handoff，不会创建发送动作。无效 JSON、截断响应、超时、目标或上下文变化不会变成“发送成功”。VERIFIED_OUTGOING 表示观察到新的匹配己方消息，不表示平台送达或已读。

## 4. 当前配置方式（2026-10-03 更新）

普通用户双击项目根目录 `打开FoxBot设置.command`，在本机页面添加多个 AI 接口，直接填写 API Key 并保存。配置使用 v2 JSON、任务使用普通 SQLite；无需下面历史版的钥匙串步骤。详见[本地设置说明](LOCAL_SETTINGS.md)。

当前命令使用 `CONFIG="$(target/debug/foxbot-host config-path)"` 取得设置页保存的文件，配置预检改用 `g3c-reply-check "$CONFIG" g2d-test --check-config`。arm/once 的 CONFIG 参数也改为这份文件。每个新任务固定准备时的默认接口，切换默认接口不会切换现有任务；选定接口改变后重启会拒绝旧任务而不是重发。

微信重启会改变易失的 application-session fingerprint。确认仍是同一账号、同一测试私聊且输入框为空后，使用 `g3c-rebind-session target/macos-probe/debug/foxbot-macos-send g2d-test --confirm-same-account`。它只读取两次当前窗口，要求会话指纹不变且与此前证据至少有两条连续消息重叠，再单独保存 G3c 运行会话绑定；不会改写 G2 baseline/verified 证据、调用模型、回填或发送。

普通构建默认无加密功能；只有读取旧 v1 加密记录时才显式启用 `--features encrypted-ledger`。以下保留旧配置格式作为历史兼容说明，**不是普通用户的操作步骤**。

## 4a. 旧 v1 配置准备（历史兼容）

本轮检查了本项目 config/configs/local/artifacts/examples/target/g2d-real 及 FoxBot 专用用户配置目录，仅找到示例，没有发现可直接使用的真实 AI 配置。不要借用其他项目或其他软件的 API Key。

模板：[examples/g3c2-reply.json](../../examples/g3c2-reply.json)。填入完整 HTTP 端点和实际 model，API Key 使用系统凭据引用；非秘密配置也要求 0600。占位端点/模型不允许作为有效配置进入联调。

```bash
mkdir -p artifacts/local
chmod 700 artifacts/local
install -m 600 examples/g3c2-reply.json artifacts/local/g3c2.json
# 编辑 artifacts/local/g3c2.json：填写 http.endpoint 和 http.model。
```

构建最终使用的二进制后，为本次联调初始化一个新的命名账本 key；已有条目不覆盖：

```bash
target/debug/foxbot-host init-key foxbot-g3c2-ledger --confirm-keychain-write
python3 -c 'import getpass,sys; sys.stdout.write(getpass.getpass("API Key: "))' \
  | target/debug/foxbot-host set-token foxbot-g3c2-api --confirm-keychain-write
```

上面通过 stdin 交给宿主，API Key 不进入 JSON、命令参数或仓库。无认证的本地业务测试服务可显式设 token=null，但不能因此绕过服务本身认证。

开发二进制重建后，旧 Keychain 条目的权限可能需要重新授权；本期保留无交互失败，不能覆盖旧账本 key 或降级明文来继续。配置好凭据之后不要无故重建二进制再执行同一验收。

## 5. 开发联调命令（下列路径为旧 v1 示例）

无需模型配置即可验证私有原生读链（不建立新基线、不发送）：

```bash
target/debug/foxbot-host g3c-reply-read-check \
  target/macos-probe/debug/foxbot-macos-send g2d-test --allow-native-read
```

配置与凭据预检（不请求模型；AVAILABLE 不等于端点已联网验证）：

```bash
target/debug/foxbot-host g3c-reply-check \
  artifacts/local/g3c2.json g2d-test --allow-keychain-read
```

如微信自上次验收后重新启动，先执行受控重绑定：

```bash
target/debug/foxbot-host g3c-rebind-session \
  target/macos-probe/debug/foxbot-macos-send g2d-test \
  --confirm-same-account
```

原测试私聊保持前台、滚动到底部，使用一个尚未使用的 RUN：

```bash
target/debug/foxbot-host g3c-reply-arm artifacts/local/g3c2.json \
  target/macos-probe/debug/foxbot-macos-send g2d-test ai001 --allow-native-read
```

确认输出 ARMED_WAITING_FOR_NEW_MESSAGE 后，由测试对端发一条新消息，再执行：

```bash
target/debug/foxbot-host g3c-reply-once artifacts/local/g3c2.json \
  target/macos-probe/debug/foxbot-macos-send g2d-test ai001 \
  --allow-network --allow-single-test-send
```

成功后再次执行同一个 once 命令可验证不重复生成/发送。不要通过再次 arm 或改 RUN 消除旧 UNKNOWN。Ctrl-C 在等待或模型请求时取消，不把本地取消当成服务端已经取消；原生调用期间取消在该有界调用返回后处理。

## 6. 本地证据与下一步

旧 v1 联调状态位于 `target/g2d-real/<SESSION>/g3c-2/<RUN>/`，仍使用 SQLCipher。当前 v2 本地配置把任务保存到配置同级的 `runs/<SESSION>/<RUN>/`，上下文和完整 AI 回复使用普通 SQLite，无需账本密钥。run.json 保存选定接口 ID、配置摘要、绑定、历史签名和阶段，不保存聊天原文和 API Key；不同存储模式不隐式转换已有任务。公开报告只包含状态、model job 次数和 native 调用计数；BusinessV1 的反馈请求次数不混作模型生成次数。

验收分别记录：离线真实 HTTP 协议＋合成 worker、真实微信只读检查、真实模型＋真实发送。前两项不能代替第三项。待真实配置可用，先完成一条短回复的端到端验收，再扩展多行/长文本，最后进入持续值守；不扩展 IME 或人工共编专项。
