# FoxBot 文档导航

当前文档基线：**v0.1 / 2026-09-28**。这是设计阶段的版本，不是应用发布版本。

当前新增 G2b-3 持久 Vision OCR worker 和只读 MessageSnapshot，保留 G1 集成、G2a、G2b-1/2。微信 4.1.13 已在同一预热 worker 内连续完成真实单窗口 OCR；输出只含脱敏统计，解析仍为启发式、会话身份 UNVERIFIED。QQ 未运行，没有真实模型联调、真实发送、安装包或长稳结论。

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

已执行的检查或测试回执保存到 `docs/acceptance/receipts/`，清单本身不作为累计通过报告。新增回执时使用稳定文件名，标记实际被检提交；文档回执的提交与被测程序提交可以不同，不能混写。

## 3. 阅读与实施顺序

先读设计基线第 1–3 节明确边界，再看适配矩阵核对目标组合；实现共享核心前阅读数据契约、回复服务与可靠发送章节。写代码时同步选取验收案例，不在功能完成后补造门禁。

平台开发从只读探针开始，再分别验证草稿、回填、发送和回执。测试人员按清单的阶段选择案例，复制模板形成回执。发布支持声明只来自相应环境与运行模式的通过记录。

## 4. 状态和证据规则

- `PLANNED`：设计目标，未实现。
- `IMPLEMENTED`：存在实现，不表示已经真机通过。
- `NOT_RUN / PASS / FAIL / BLOCKED / NA`：案例结果；NA 附理由，BLOCKED 不是 PASS。
- `ACCEPTED`：某提交、设备/系统、应用版本、聊天类型、通道和模式已有通过回执。

当前六个首批适配器均为 `PLANNED / NOT_RUN`；Android 微信是 `PROBE_ONLY`。不能因当前会话自动回复通过就启用多会话值守，不能因一端成功把其他端一并标绿。

## 5. 文档变更规则

本轮整理的是已经讨论过的产品范围。技术分工和协议细节是工程工作基线，可经有依据的变更调整，不假称每项细节都已被用户单独批准。

修改产品范围、数据语义、发送权限或恢复规则时，更新设计基线、矩阵、受影响案例与变更理由；单纯补一个支持表不构成适配完成。

参考仓库升级、目标应用升级、OCR 模型/字典/预后处理升级都要固定新版本并标记受影响回归，不能静默覆盖旧证据。上游代码与模型资源的复用必须更新来源台账。

所有公开文档、回执与附件必须脱敏；密钥通过本地安全配置提供。测试故障、未覆盖范围和实际运行时长必须保留，不得以“CI 绿”“编译成功”替换真机验收。

## 6. 当前阶段和下一项

G2b-3 已跑通真实微信的持久 worker OCR，并形成只读 MessageSnapshot；下一项应补稳定会话身份、真实消息人工标注对照和运行中 QQ 样本，再决定如何桥接到核心 Observation。真实发送继续保持关闭。

目标应用版本与定制服务脱敏样例可在联调时补充，不阻塞模拟开发；未取得对应证据之前，不承诺真实客户端兼容性。根目录 scripts/check_docs.py 可直接检查本地 Markdown 文件目标、JSON 示例和案例引用，不联网检查外部链接。
