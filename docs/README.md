# FoxBot 文档导航

当前文档基线：**v0.1 / G3c-1 当前私聊单条原生发送 / 2026-10-01**。这是工程能力基线，不是应用发布版本。

macOS 微信 G2 真实读取链已完成 Freeze；G3a 已证明测试级草稿回填和 OCR 回读可行。用户已明确实际使用不会人工操作聊天软件，G3b 双门禁因此改为无人值守独占契约：不要求 IME、候选窗或最近键鼠活动证据，保留会话、内容、权限、执行权与防重复发送。IME Spike 归档。G3c-1 已实现独立的单条原生发送与新己方消息核对；固定测试回复的真实验收单独记录，完整 AUTO_REPLY 尚未接通。

## 1. 三份主文档

| 文档 | 用途 | 主要读者 |
| --- | --- | --- |
| [设计基线](design/BASELINE.md) | 目标、模式、分层、消息/回复契约、可靠发送、技术方向、阶段门禁 | 产品、开发、后续接手的 Agent |
| [适配能力矩阵](adapters/CAPABILITY_MATRIX.md) | 六个首批平台组合、独立探测项、能力缺口与案例映射 | 平台开发、测试 |
| [验收清单](acceptance/ACCEPTANCE_CHECKLIST.md) | 从文档检查到模拟故障、真机收发、多会话值守和发布的固定案例 | 开发、测试、发布负责人 |

## 2. 配套文档

| 文档 | 用途 |
| --- | --- |
| [验收回执模板](acceptance/RECEIPT_TEMPLATE.md) | 写清实际测试提交、环境、步骤、证据、结果与未覆盖范围 |
| [上游与技术来源审计](references/UPSTREAM_AUDIT.md) | 三端参考仓库固定提交、源码证据、许可证和复用边界 |
| [项目入口](../README.md) | 产品范围与开发顺序 |
| [G1a 开发说明](development/G1_RUNTIME.md) | 实际运行命令、模拟协议、持久化边界、测试映射与 G1b 缺口 |
| [G1a 本地验收回执](acceptance/receipts/2026-09-28-g1a-simulation.md) | 实测代码提交、48 项回归、进程退出恢复及未覆盖范围 |
| [G1b HTTP 开发说明](development/G1_HTTP_PROVIDER.md) | 配置、协议、取消/幂等、回执补偿、本机测试与生产边界 |
| [G1b 本地 HTTP 回执](acceptance/receipts/2026-09-28-g1b-http.md) | 被测提交、85 项回归、HTTP 子进程中断与补偿 smoke |
| [G1c 持续宿主与安全配置](development/G1_HOST_SECURITY.md) | 常驻命令、暂停代际、设备锁、钥匙串、SQLCipher 与未覆盖边界 |
| [G1c 本地验收回执](acceptance/receipts/2026-09-28-g1c-host.md) | 103项回归、关闭加密专项、宿主进程/钥匙串探针和实际限制 |
| [G1 集成对账](development/G1_INTEGRATION.md) | 一键门禁、tick输入竞态修复、已覆盖子集与剩余阻塞 |
| [G2a macOS只读探针](development/G2_MACOS_PROBE.md) | 限定目标、默认脱敏、构建运行、超时和权限边界 |
| [G1集成 / G2a 验收回执](acceptance/receipts/2026-09-29-g1-integration-g2a.md) | 固定被测提交、106项Rust及只读探测的实际边界 |
| [G2b-1 单窗口与 OCR](development/G2_WINDOW_OCR.md) | 独立截图入口、Vision 合成测试、选窗约束与实际阻塞 |
| [G2b-1 验收回执](acceptance/receipts/2026-09-29-g2b-window-ocr.md) | 固定被测提交、集成回归和原生微信/QQ未完成边界 |
| [G2b-2 窗口身份绑定](development/G2_WINDOW_BINDING.md) | 微信多进程/Space窗口绑定、真实单窗口捕获证据和 OCR 冷启动边界 |
| [G2b-2 验收回执](acceptance/receipts/2026-09-29-g2b2-window-binding.md) | 固定代码提交、真实微信窗口捕获、同框消歧与真实 OCR 超时边界 |
| [G2b-3 OCR Worker / MessageSnapshot](development/G2_OCR_WORKER_MESSAGE_SNAPSHOT.md) | 持久预热、请求级超时、Vision ROI、气泡方向/发言人启发式和真实 OCR 边界 |
| [G2b-3 验收回执](acceptance/receipts/2026-09-29-g2b3-ocr-worker.md) | 固定代码提交、真实微信持久 worker OCR、脱敏 MessageSnapshot 与未覆盖边界 |
| [G2c 会话身份与 Rust Host](development/G2C_IDENTITY_HOST.md) | 应用会话/会话指纹、显式绑定、跨帧连续性、ground-truth 门禁和 worker 生命周期 |
| [G2c 验收回执](acceptance/receipts/2026-09-29-g2c-identity-host.md) | 固定代码提交、真实微信两帧身份稳定、Rust host 生命周期与 PROVISIONAL 边界 |
| [G2d Observation Bridge](development/G2D_OBSERVATION_BRIDGE.md) | PrivateMessageSnapshot → Runtime::ingest、两阶段游标提交、背压重试和真实验收边界 |
| [G2d 验收回执](acceptance/receipts/2026-09-29-g2d-observation-bridge.md) | 固定代码提交、合成 Runtime Bridge PASS 与真实微信 BLOCKED 边界 |
| [G2d 真实验收工作流](development/G2D_REAL_ACCEPTANCE.md) | 私有采样、人工 ground-truth、显式 Binding、baseline、单条 incoming 验证与状态引导器 |
| [G2d Readiness 回执](acceptance/receipts/2026-09-29-g2d-real-readiness.md) | 固定提交的真实私有采样、0600 权限、自我验收防护与 BLOCKED 边界 |
| [G2 Freeze 回执](acceptance/receipts/2026-09-30-g2-freeze.md) | 6 类真实 ground-truth、real Observation PASS、最终门禁和 G2/G3 边界 |
| [G3a Draft Writer](development/G3A_DRAFT_WRITER.md) | macOS 微信测试级草稿检测、前台/身份门禁、Unicode 回填、OCR 回读与 no-send 边界 |
| [G3a Draft Writer 回执](acceptance/receipts/2026-10-01-g3a-draft-writer.md) | 已有草稿拒绝覆盖、空草稿真实写入回读和 send_attempted=false 的真机证据 |
| [G3b Safe Send Gate](development/G3B_SAFE_SEND_GATE.md) | 无人值守双门禁、GUI ownership、revision/outbox 与 native no-send facts |
| [G3b 无人值守独占执行](development/G3B_UNATTENDED_EXECUTION.md) | 用户运行前提、移除 IME/人工活动门禁、开发暂停交接和 G3c 路线 |
| [G3b 无人值守调整回执](acceptance/receipts/2026-10-01-g3b-unattended-execution.md) | 固定代码提交，13/13 集成检查、Rust 137 / Python 61 / Swift 97；未执行真实发送 |
| [G3c-1 当前私聊单条真实发送](development/G3C1_SINGLE_REAL_SEND.md) | Runtime 加密待发任务、原生单次写入/点击、回执序列关联与只读重放核对 |
| [G3c-1 收口回执](acceptance/receipts/2026-10-01-g3c1-closeout.md) | 新任务自动 VERIFIED_OUTGOING，同 RUN 重放零原生调用；Keychain 超时修复、152/61/116 回归 |
| [G3c-2 真实消息与 AI 回复联调](development/G3C2_REAL_REPLY.md) | 单次 arm→新消息→HTTP模型→原生发送确认；配置、幂等与未实测范围 |
| [G3c-2 联调准备回执](acceptance/receipts/2026-10-01-g3c2-real-reply-readiness.md) | 166/61/117 回归与真实私聊只读通过；实际模型和真实自动回复待配置 |
| [G3c-1 实现旧回执](acceptance/receipts/2026-10-01-g3c1-single-real-send.md) | 先前实现回归通过、测试会话未就绪时的历史记录 |
| [G3c-1 首次真实发送与回执修复](acceptance/receipts/2026-10-01-g3c1-real-attempt-and-receipt-fix.md) | 实际发送一次、同 RUN 不重发；修复漏裁/时间分隔并通过 149/61/113 回归；保留当时 UNKNOWN/超时历史，当前收口见新回执 |
| [G3b Safe Send Gate 旧回执](acceptance/receipts/2026-10-01-g3b-safe-send-gate.md) | 历史 Core/Host PASS 与旧 IME 条件下的 native BLOCKED；不是新版 Gate 回执 |
| [G3b IME Evidence Spike（归档）](development/G3B_IME_EVIDENCE_SPIKE.md) | 独立诊断实验及证据边界，已退出正常执行路径，不再阻塞 G3c |
| [G3b IME Spike 旧回执](acceptance/receipts/2026-10-01-g3b-ime-evidence-spike.md) | 当时的真实采样和实验；保留原结果，开发阻断结论已被独占契约取代 |

已执行的检查或测试回执保存到 `docs/acceptance/receipts/`，清单本身不作为累计通过报告。新增回执时使用稳定文件名，标记实际被检提交；文档回执的提交与被测程序提交可以不同，不能混写。

## 3. 阅读与实施顺序

先读设计基线第 1–3 节明确边界，再看适配矩阵核对目标组合；实现共享核心前阅读数据契约、回复服务与可靠发送章节。写代码时同步选取验收案例，不在功能完成后补造门禁。

平台开发从只读探针开始，再分别验证草稿、回填、发送和回执。测试人员按清单的阶段选择案例，复制模板形成回执。发布支持声明只来自相应环境与运行模式的通过记录。

## 4. 状态和证据规则

- `PLANNED`：设计目标，未实现。
- `IMPLEMENTED`：存在实现，不表示已经真机通过。
- `NOT_RUN / PASS / FAIL / BLOCKED / NA`：案例结果；NA 附理由，BLOCKED 不是 PASS。
- `ACCEPTED`：某提交、设备/系统、应用版本、聊天类型、通道和模式已有通过回执。

MC-WX 的 **G2 current-session 读取链**已有本机真实 PASS；G3a 已证明测试级 C06 fill。G3b 按独占运行修订；旧 IME 阻断不再适用，但不能把旧真机记录或新单测 READY 当作新版真实收发 PASS。G3c-1 受限短文本 C07/C08 已取得当前测试私聊真实闭环回执，同一任务重放零原生调用；其余首批适配器仍未取得对应真实回执，Android 微信是 `PROBE_ONLY`。

## 5. 文档变更规则

本轮整理的是已经讨论过的产品范围。技术分工和协议细节是工程工作基线，可经有依据的变更调整，不假称每项细节都已被用户单独批准。

修改产品范围、数据语义、发送权限或恢复规则时，更新设计基线、矩阵、受影响案例与变更理由；单纯补一个支持表不构成适配完成。

参考仓库升级、目标应用升级、OCR 模型/字典/预后处理升级都要固定新版本并标记受影响回归，不能静默覆盖旧证据。上游代码与模型资源的复用必须更新来源台账。

所有公开文档、回执与附件必须脱敏；密钥通过本地安全配置提供。测试故障、未覆盖范围和实际运行时长必须保留，不得以“CI 绿”“编译成功”替换真机验收。

## 6. 当前阶段和下一项

**G3c-1 当前测试范围已收口**：新任务 853208 从 PREPARED 自动到 VERIFIED_OUTGOING；重放读取账本后返回，不再调用 native。Keychain 超时已定位并改为明确的无交互失败，历史 OCR 空格波动已与正文精确匹配分离；旧 UNKNOWN 保留且不重发。G3c-2 已接通新 incoming / HTTP ReplyProvider 的单次代码入口，目前仅找到示例配置；真实模型与聊天端到端验收待真实端点、模型及凭据就绪。短单行限制仍保留，不把未验收的长文/多行算作支持。持续值守、多会话另行推进；不新增 IME 或人工并发输入专项。

目标应用版本与定制服务脱敏样例可在联调时补充，不阻塞模拟开发；未取得对应证据之前，不承诺真实客户端兼容性。根目录 scripts/check_docs.py 可直接检查本地 Markdown 文件目标、JSON 示例和案例引用，不联网检查外部链接。
