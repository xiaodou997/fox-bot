# 上游与技术来源审计

- 编号：FB-SOURCES-001
- 核对日期：2026-09-28
- 关联：[设计基线](../design/BASELINE.md) · [适配矩阵](../adapters/CAPABILITY_MATRIX.md)

第 1–7 节记录 G0 设计阶段的固定参考来源和当时边界，不是参考聊天项目实现已迁入或 FoxBot 已通过真机聊天测试的声明。后续实际引入的基础库见第 8 节；不能将 G0 的“仅文档”状态当作当前仓库无代码。

## 1. 固定快照

| 来源 ID | 仓库 | 分支 | 核对时完整提交 SHA |
| --- | --- | --- | --- |
| S-A | jev-chat/jev-chat-jarvis | main | 5feeb8ead9cf9956ec25f112f15dd8b7b369fa41 |
| S-M | jev-chat/jev-chat-jarvis-mac | master | 01312b05655566a4ba4479899373a6826e95867d |
| S-W | jev-chat/jev-chat-windows | main | 946d3d1409b5f340d2b2c2e2a067d4c143638d5a |

上游分支会继续变化，下面使用固定提交链接。重新审计时新增日期和差异，不将当前 main/master 内容冒充这份快照。

## 2. S-A：Android

- [ChatCaptureService.kt](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/app/src/main/java/com/jev/probe/capture/ChatCaptureService.kt)：注册 QQAdapter、XAdapter、FeishuAdapter；微信入口被主动排除。主采集路径围绕前台窗口；注释与写入路径明确只填入、不发送。
- [ChatAppAdapter.kt](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/app/src/main/java/com/jev/probe/capture/ChatAppAdapter.kt)：QQ 的 resource-id/text，X 的 contentDescription，飞书气泡几何和状态解析。仍存在的 WeChatAdapter 不能证明当前服务使用它。
- [ScreenCapture.kt](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/app/src/main/java/com/jev/probe/capture/ocr/ScreenCapture.kt)：系统截图、窗口/显示坐标转换、限频、退避、超时和缓冲释放。
- [MlKitOcr.kt](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/app/src/main/java/com/jev/probe/capture/ocr/MlKitOcr.kt)及[OcrEngine.kt](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/app/src/main/java/com/jev/probe/capture/ocr/OcrEngine.kt)：本地 OCR 路径。不能因存在 VisionClient 就宣称当前实际 OCR 是视觉大模型。
- [GuardedInputWriter.kt](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/app/src/main/java/com/jev/probe/capture/GuardedInputWriter.kt)：重新解析目标、写入检查与降级序列，可借鉴其执行前后校验思想。

可借鉴：适配器分发、空窗口与无正文的区别、局部 OCR、会话失效、限频和读回验证。不可直接继承：实际设备支持声明、旧版本 ID 稳定性、微信可用性、伪装服务身份、自动发送和多会话值守能力。

Android 微信禁用的原因在上游注释中由作者描述；本项目未独立验证其对所有系统/应用版本的适用性，不保证使用任何特定技术就没有平台风险。

## 3. S-M：macOS

- [registry.py](https://github.com/jev-chat/jev-chat-jarvis-mac/blob/01312b05655566a4ba4479899373a6826e95867d/src/apps/registry.py)：实际注册 CaptureApp 与 AXApp；前台查询失败的 UNKNOWN 与确实离开分开处理。
- [capture_app.py](https://github.com/jev-chat/jev-chat-jarvis-mac/blob/01312b05655566a4ba4479899373a6826e95867d/src/apps/capture_app.py)：微信截图路径的适配封装，转调 perception 与 fill。
- [perception.py](https://github.com/jev-chat/jev-chat-jarvis-mac/blob/01312b05655566a4ba4479899373a6826e95867d/src/perception.py)：窗口与截图/OCR 路径，包含几何与内容解析假设，以及捕获阻塞隔离的说明。
- [ax_app.py](https://github.com/jev-chat/jev-chat-jarvis-mac/blob/01312b05655566a4ba4479899373a6826e95867d/src/apps/ax_app.py)：QQ 的 AX 文本和编辑器解析；AXReader 抽象便于内存树测试；读取失败不当作空草稿；填入不等于发送。
- [README.md](https://github.com/jev-chat/jev-chat-jarvis-mac/blob/01312b05655566a4ba4479899373a6826e95867d/README.md)：区域校准、权限与已知验收边界。若概述与注册路径的覆盖描述不同，以对应源码路径和明确版本证据分别记录。

可借鉴：原生属性读取、有限遍历、前台身份、坐标校准、不可读草稿保护和测试替身。不可直接继承：所有主题/版本可读、长期稳定性、当前会话之外的值守、自动发送验收。

## 4. S-W：Windows

- [README.md](https://github.com/jev-chat/jev-chat-windows/blob/946d3d1409b5f340d2b2c2e2a067d4c143638d5a/README.md)：作者记录目标微信窗口使用 WGC＋RapidOCR，自绘界面的 UIA 正文不可用。该观察限定于上游测试场景，不外推到所有软件与版本。
- [app/fill.py](https://github.com/jev-chat/jev-chat-windows/blob/946d3d1409b5f340d2b2c2e2a067d4c143638d5a/app/fill.py)：剪贴板、窗口/输入区定位、鼠标和粘贴输入，函数明确停止在填入，不按发送。

可借鉴：采集/OCR 分层、窗口几何、调试可观察性和原生资源处理。固定偏移、剪贴板写入和前台激活行为不能未经重新设计直接用于无人值守；需要 FoxBot 独立的目标、草稿、执行权与效果验证。

## 5. 许可证与复用记录

本次核对的三个代码仓库均声明 MIT：

| 来源 | 固定许可证 | 版权声明主体 |
| --- | --- | --- |
| S-A | [LICENSE](https://github.com/jev-chat/jev-chat-jarvis/blob/5feeb8ead9cf9956ec25f112f15dd8b7b369fa41/LICENSE) | Finderchangchang and the jev-chat contributors |
| S-M | [LICENSE](https://github.com/jev-chat/jev-chat-jarvis-mac/blob/01312b05655566a4ba4479899373a6826e95867d/LICENSE) | eatmoreduck |
| S-W | [LICENSE](https://github.com/jev-chat/jev-chat-windows/blob/946d3d1409b5f340d2b2c2e2a067d4c143638d5a/LICENSE) | rezoch340 and the jev-chat contributors；另含来自 Android 项目的版权声明 |

复用代码或实质部分时保留适用的版权和许可通知，同时检查对应目录和 NOTICE。模型权重、依赖、图像和品牌资产分别审核，不能用仓库顶层 MIT 覆盖所有外部资源。开源代码许可也不是第三方客户端对自动化使用的授权或零风险保证。

本轮没有为 FoxBot 选择或替换自身 LICENSE；在实际引入代码和发布前单独确定。当前复用台账为空，设计参考不冒充已迁入的组件。

未来每次复用至少记录：

```text
upstream_repository:
upstream_commit:
upstream_path:
local_path:
license_and_notice_paths:
change_summary:
model_or_asset_license_if_any:
review_and_validation_receipt:
```

## 6. 平台与 OCR 技术资料

以下是设计候选和平台边界的第一方资料，不表示依赖版本已经冻结。查阅日期为 2026-09-28。

| 来源 ID | 资料 | 本基线使用范围 |
| --- | --- | --- |
| T-01 | [ML Kit Android Text Recognition](https://developers.google.com/ml-kit/vision/text-recognition/v2/android) | 区分构建时打包模型与通过 Google Play Services 动态下载；Android OCR 候选。 |
| T-02 | [Android 通知与直接回复](https://developer.android.com/develop/ui/compose/notifications/create-notification) | 平台支持通知回复机制；不证明指定聊天应用已经提供或正确暴露相关动作。 |
| T-03 | [Apple 运行时沙盒安全](https://support.apple.com/guide/security/security-of-runtime-process-sec15bfe098e/web) | iOS 普通应用跨应用访问受限；不规划与 Android 等价的任意应用控制承诺。 |
| T-04 | [Microsoft UI Automation 线程问题](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading) | 原生自动化与 UI 线程隔离、调用生命周期需要专门处理。 |
| T-05 | [RapidOCR](https://github.com/RapidAI/RapidOCR) | 本地 OCR 参考实现与基准候选；正式引入时固定提交并核对运行时/模型许可。 |
| T-06 | [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) | 轻量检测/识别模型候选；具体版本、字典、预后处理和效果均待选型测试。 |
| T-07 | [ncnn Android PP-OCR 示例](https://github.com/nihui/ncnn-android-ppocrv5) | 原生移动端部署的比较路线，不是现成聊天适配器。 |

不将此前讨论中的模型大小、版本宣传或通用数据集分数直接写成 FoxBot 性能承诺。模型文件大小、完整安装包大小、运行内存和端到端延迟是不同指标，均需要独立记录。

## 7. 本轮未执行事项

未构建三个参考项目，未安装客户端，未运行 OCR 基准，未打开真实聊天账号，未填入或发送任何消息。以上来源用于文档与设计，不构成软件功能回执。

## 8. G1c 实际引入的基础依赖（2026-09-28）

本节记录 G1c 的工程依赖，不改变前述三端参考仓库快照；没有复制其聊天适配代码。精确解析与 checksum 以 Cargo.lock 为准。

| 组件 | 当前解析版本 | 用途与依据 |
| --- | --- | --- |
| rusqlite / libsqlite3-sys | 0.40.2 / 0.38.2 | [feature 文档](https://docs.rs/crate/rusqlite/0.40.2/features)；host 默认启用 bundled-sqlcipher-vendored-openssl，独立加密入口。 |
| SQLCipher API | 随 libsqlite3-sys 的 bundled 源码构建 | [上游 API](https://www.zetetic.net/sqlcipher/sqlcipher-api/)；key 必须先于模式读取，读库确认密钥，不自动将明文库当密文处理。 |
| openssl-src / openssl-sys | 300.6.1+3.6.3 / 0.9.117 | SQLCipher 构建依赖，版本来自锁文件；不是声明取得 FIPS 或商业认证。 |
| security-framework | 3.7.0 | [macOS 密码 API](https://docs.rs/security-framework/3.7.0/security_framework/passwords/index.html)；固定命名空间的凭据获取/创建，不枚举既有凭据。 |
| dirs | 6.0.0 | 同 OS 用户固定执行锁根目录，不依赖用户选择的账本目录。 |
| zeroize | 1.9.0 | Secret 与原始数据库 key 临时缓冲区清理；不宣称所有依赖内部副本均被清零。 |
| getrandom | 0.4.3（直接使用） | 操作系统随机源生成账本密钥和隔离探针名称；锁文件另有 0.2.17 的传递依赖。 |

首次 SQLCipher 构建、加密读写和 macOS 随机凭据探针的实际结果记录于 G1c 回执；其他平台凭据、ACL、发布签名及完整许可证分发清单仍需发布前核对。安装了库或 API 可编译，不代表其所有平台能力已验收。FoxBot 自身 LICENSE 仍未选定，不因引用 MIT 上游或上述库自动取得统一许可证结论。
