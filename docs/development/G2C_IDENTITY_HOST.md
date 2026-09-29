# G2c：会话身份、Ground Truth 与 Rust 原生读取宿主

- 日期：2026-09-29
- 状态：实现已具备；真实微信 worker 私有快照可由 Rust host 获取，但真实 ground-truth 与显式会话 Binding 尚未建立，因此真实 Observation 桥接保持 PROVISIONAL。
- 前置：[G2b-3 OCR Worker / MessageSnapshot](G2_OCR_WORKER_MESSAGE_SNAPSHOT.md)
- 关联：[设计基线](../design/BASELINE.md) · [能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)

## 1. 身份不是一个标题哈希

G2c 将身份拆成四层：

1. `Binding.key.account_binding / app_instance / conversation`：由 FoxBot 配置的稳定业务身份。
2. `identity_epoch`：账号、绑定或确认状态变化时使旧任务失效。
3. `application_session_fingerprint`：目标 bundle + 进程 launch time 的本地 SHA-256；应用重启后变化，旧跨帧轨迹失效。
4. `conversation_fingerprint`：规范化当前标题后的本地 SHA-256，只作为视觉证据，不单独成为 ConversationKey。

用户显式把当前私有快照绑定到稳定 Binding 后，身份来源标记为 `configured_wechat_title_continuity_v1`。如果配置中两个会话使用同一个视觉指纹，直接拒绝，不选择其中一个。

同一标题也不足以继续路由。跨帧必须至少有两条完整消息满足“上一帧 suffix == 下一帧 prefix”；只有一条“好的”之类的公共文本重叠会返回 `AMBIGUOUS`。如果初始基线不足两条，只扩大 historical baseline，不产生新工作。

应用进程重启会改变 application-session 指纹并清理旧轨迹。**同一进程内部登出/切换账号目前没有可靠系统级自动信号**，因此仍要求外层在账号变化时显式 invalidate/rebind；不能把当前实现描述成自动识别所有登录变化。

## 2. MessageSnapshot → Observation 门禁

`NativeObservationBridge` 只有同时满足以下条件才会产生只读 Observation：

- ground-truth 结果已被接受，且 strategy 与快照相同；
- 当前 application-session 与显式 binding 一致；
- 当前 conversation fingerprint 有唯一显式 Binding；
- 快照只存在允许的 `HEURISTIC_REGION` 不确定性；
- 消息方向不是 UNKNOWN，正文完整；
- 群聊 incoming 消息存在 sender fingerprint；
- 已形成至少两条跨帧消息连续性。

首帧永远只产生 `historical=true` baseline。无重叠、身份变化、未知方向或不完整消息不会成为新工作。消息 canonical id 使用 conversation fingerprint + identity epoch + 本地顺序生成，不按正文哈希，因此连续两条完全相同的“好的”仍可以是两条消息。

本模块只构造 Observation；没有增加发送能力。真实微信目前缺少 accepted ground-truth 和显式 Binding，所以探针固定显示 PROVISIONAL。

## 3. Ground-truth 验收框架

原始人工标注可能包含聊天正文，只允许保存在 Git 忽略目录：

```text
target/g2c-groundtruth/
```

运行：

```bash
python3 scripts/g2c_ground_truth.py target/g2c-groundtruth/<private-fixture>.json
```

输入覆盖 expected/observed 的文本、方向和 sender 是否存在；输出**不包含正文**，只包含计数。接受门槛：

- 至少 6 个 case、24 条人工标注消息；
- 必须覆盖 private、group、duplicate_text、numeric、multiline、reference；
- direction / sender / message-count error 必须为 0；
- 文本误差不超过 2%；
- strategy 和 revision 必须显式记录。

本轮使用纯合成私有 fixture 验证框架可产生 accepted 结果；**没有创建或提交真实微信 ground-truth**，因此真实策略仍没有可用于生产放行的 accepted 记录。

## 4. Rust NativeReadHost

`foxbot-host` 新增独立原生读取管理线程：

- 默认 PAUSED；
- resume 时启动并 warmup OCR worker；
- pause 关闭 worker，下一次 resume 重新预热；
- 每个 worker 请求有硬时限；崩溃或无效响应 fail-closed，下一次读取重新启动并预热；
- JSONL 请求/响应有大小上限，worker stderr 不透传；
- stdout reader 线程由 WorkerProcess 持有并在结束时 join；
- 命令队列有界，满时立即 `Backpressure`，不无限堆积读取；
- lifecycle Stop 使用可靠入队，不因为队列瞬间满而静默丢失；
- DeviceOwner Drop 显式 unlock，但保留同一锁文件 inode。

这些能力使用真实子进程和 fake worker 回归，不依赖模拟字段改状态。

## 5. 真实微信只读探针

开发入口：

```bash
target/debug/foxbot-host native-read-probe \
  target/macos-probe/debug/foxbot-macos-ocr \
  --allow-native-read
```

它会由 Rust host 启动/预热 worker，连续读取两次，并在进程内比较 application-session 与 conversation fingerprint。公开 JSON 不输出任何指纹值或正文，只输出：

- 两种 fingerprint 是否 `STABLE_TWO_READS`；
- message count；
- partial reasons；
- worker starts / successes / failures；
- `write_or_send_operations=0`；
- Observation bridge 状态。

本轮真实微信 4.1.13 已完成该链路；快照进入 Rust 内存，但因为没有真实 ground-truth 与配置 Binding，状态保持：

```text
PROVISIONAL_REQUIRES_CONFIGURED_IDENTITY_AND_ACCEPTED_GROUND_TRUTH
```

这是预期的 fail-closed 行为，不是功能失败。

## 6. 下一步与仍未关闭项

G2c 后续真实验收需要测试人员使用专用聊天账号/会话建立私有 ground-truth，并显式绑定对应稳定 ConversationKey。完成后才可验证真实新增消息是否生成正确 Observation。

另外仍需：

- 同进程内登出/换号的自动失效信号；
- 真实群聊 sender、引用、数字金额、多行、重复短句的人工对照；
- Rust 长期调度中的周期读取、pause/stop 与背压策略；
- QQ 运行中样本；
- macOS 26、多显示器/Spaces/睡眠恢复。

真实写入与发送继续属于 G3，本阶段保持完全关闭。
