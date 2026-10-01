# G3c-2 单次真实消息与 AI 回复联调准备

- 日期：2026-10-01
- 代码提交：`997d56c0a87e70f28ba3fff08038082a44011254`
- 基线 main：`99e36cb3b502f3d19409ea090a22ba9ae272bee1`
- 分支：`feat/g3c2-real-reply`
- 本机环境：macOS 27.0 / arm64 / 微信 4.1.13。
- 结论：**单次联调实现与离线协议回归 PASS；原测试聊天的私有原生读检查 PASS；真实 AI 服务与真实发送端到端 NOT_RUN，等待实际服务配置。**
- 说明：[G3c-2 开发与配置](../../development/G3C2_REAL_REPLY.md) · [G3c-1 真实发送基线](2026-10-01-g3c1-closeout.md)

## 1. 实际实现

新增 read-check / config-check / arm / once 入口。arm 在加密账本中记录当前历史，只处理此后出现的一条对方消息。真实正文由原生 read 私有 IPC 提供，不再使用测试 fixture 作为生产触发。原文、方向、完整性与回执签名来自同一解析投影，宿主验证对应关系。

once 使用现有 HttpReplyService 和 Runtime，先落盘 generation claim，再请求模型；模型返回后重新核对原会话和上下文，发送通道同时固定到生成回复时的上下文。完成重放不再生成或发送；未知发送只读核对；生成中断不盲目调用第二次模型。BusinessV1 保留自定义服务不加系统提示词与独立反馈契约。

原生 IPC 为 v4，证据解析仍是 WECHAT_RECEIPT_V3，未重写旧回执。正文仍仅支持短单行、最多 80 UTF-16 单元；长文/多行不截断，拒绝写入并保留模型完整返回。此增量不是持续值守、多会话或跨 RUN 全局去重。

## 2. 固定代码提交回归

命令：`python3 scripts/g1_integration_check.py --with-macos-probe`。

报告：`target/g1-integration/dbdceac700fc454f97def856dc5a52fe/report.json`。

检查期间代码无变化，报告 `source_unchanged=true`、`passed=true`，13/13 检查通过：

| 检查 | 实际结果 |
| --- | --- |
| Rust 工作区与全部 targets | 166 PASS |
| Python | 61 PASS |
| Swift warnings-as-errors | 117 PASS |
| 禁用加密拒绝降级专项 | 1 PASS |
| Rustfmt / Clippy / 工具构建 / bridge、gate、HTTP、host smoke / 文档 / whitespace | PASS |

新增 14 项 Rust 回归包括：模拟新消息通过实际本机 HTTP 协议与独立 worker 完成发送；SQLCipher 重启与重放不再请求模型/发送；没有新消息零请求；自身消息与多条新增拒绝；无效/长文/多行回复零写入；模型运行期间新消息使旧回复失效；UNKNOWN 只读核对；生成阶段中断不重试；业务服务反馈；取消与配置变化；重复文本新旧区分；嵌入秘密拒绝；私有正文/摘要不符拒绝；超时不重试；no_reply/handoff 不发送。Swift 新测试保证 read 与 receipt 使用同一消息投影。

这里的 HTTP 服务是本机测试服务器，消息来自合成 worker，**不是实际模型生成，也不是微信真实自动回复**。新增入口的真实业务请求次数没有由这些测试推导。

首次定向测试有 unused import 警告；首次 Clippy 发现测试支持模块重复加载和不必要借用，均修复后重新完成完整回归。不能描述为全程零失败。

## 3. 真实微信只读检查

在原已绑定测试私聊执行：

```bash
target/debug/foxbot-host g3c-reply-read-check \
  target/macos-probe/debug/foxbot-macos-send g2d-test --allow-native-read
```

最终宿主结果：

```json
{
  "status": "PRIVATE_READ_VALIDATED",
  "frontmost": true,
  "draft_state": "EMPTY_HEURISTIC",
  "message_count": 9,
  "incoming_count": 5,
  "read_requests": 1,
  "model_requests": 0,
  "write_operations": 0,
  "send_operations": 0,
  "raw_text_included": false
}
```

此前一次宿主只读检查返回 identity/trust insufficient；未保留该帧的具体阻断细节，不能猜测原因。随后一次独立只读诊断确认应用与会话匹配、9 条消息原文和签名一致、0 个不完整条目、原文与上下文 digest 不符均为 0；再执行宿主入口取得上面的 PASS。没有改绑定、放宽条件或执行写入来解决该次失败。只证明当前帧读取可用，不证明长期稳定。

本轮没有新建真实模型任务的 arm 基线，没有调用真实模型，没有回填或发送新的聊天消息，也没有重试此前 742961/853207/853208 任务。没有创建或替换本机密钥。

## 4. 真实模型联调待办

本轮仅在本项目及 FoxBot 专用用户目录查找模型配置，找到的都是示例。未发现可直接使用的真实 endpoint/model/token 配置，也未借用其他应用或项目的凭据。

需要配置实际 HTTP 端点、模型名（通用模型）或业务服务协议，以及本机凭据引用。模板是 `examples/g3c2-reply.json`，步骤在开发说明；API Key 不写入 JSON、命令参数或仓库。

配置可用后：先预检，再 arm 记录原测试会话基线，测试对端发送一条新消息，然后 once 运行实际模型→单条发送→自动确认，并重放同 RUN 验证不重复。该项通过前不把 G3c-2 标为真实端到端 ACCEPTED。

## 5. 构建身份

2026-10-01T21:32:33+08:00 复核：

| 产物 | SHA-256 |
| --- | --- |
| foxbot-host | `efb7d16b950b0a46b822d8e5e612eacd762d23d73f4d05675149febb98d821d6` |
| foxbot-macos-send | `4c027bbff31081595b116f6e5f76f982da66347e2110d753904e12069250e60a` |

本轮没有发布版本或开启常驻自动回复。固定代码回归与真实只读的证据分开记录，文档提交与被测代码提交可以不同。
