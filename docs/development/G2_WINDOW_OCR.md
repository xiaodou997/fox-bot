# G2b-1：单窗口截图与本地 Vision OCR

- 日期：2026-09-29
- 状态：截图/OCR 实现与合成回归已落地；实际微信窗口绑定未通过，QQ 未运行。不是完整 G2b 验收或真实自动回复。
- 前置：[G2a AX 探针](G2_MACOS_PROBE.md) · [G1 集成对账](G1_INTEGRATION.md)
- 关联：[能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [总验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)
- 实际结果：[本地验收回执](../acceptance/receipts/2026-09-29-g2b-window-ocr.md)，被测代码与后续文档提交分开记录。

## 1. 与 G2a 分离的实际实现

原 `foxbot-macos-probe` / ProbeCLI / ProbeKit 保持无截图能力。新增 `foxbot-macos-ocr` / OCRCLI 和 OCRKit，仍在同一个 Swift Package 内，无新增第三方 Swift 依赖。OCRKit 提供短生命周期的 `OCRLine(text, confidence, bounds)` 与只含统计的报告；文本行没有 Codable，不默认输出或持久化。

流水线为：显式允许捕获 → 校验目标应用/权限 → 选择唯一窗口 → 校验尺寸与目标新鲜度 → ScreenCaptureKit 单窗口图像 → 本地 Vision → 再核对目标 → 仅输出脱敏统计。没有服务端请求、聊天发送、输入框写入、截图保存、图片路径导出或后台录屏循环。

当前产出是窗口 OCR 可读性，不是聊天消息。菜单、会话列表和其他界面文字也可能存在于窗口中。报告固定 `content_scope=WINDOW_NOT_CHAT`，账号与会话保持 UNVERIFIED，不能直接作为核心 Observation 或发送路由。

## 2. 构建、默认行为和显式授权

```bash
xcrun swift test --package-path native/macos-probe --scratch-path target/macos-probe -Xswiftc -warnings-as-errors
python3 scripts/macos_ocr.py --app wechat
python3 scripts/macos_ocr.py --app qq
```

不传 `--capture-and-ocr` 只查询目标运行情况与录屏权限，不调用窗口截图或 OCR。程序不启动聊天应用、不激活窗口、不修改或申请系统权限。开发工具仍以 macOS 26 为编译下限，本轮验证环境是 macOS 27.0 arm64；其他系统没有继承通过结论。

明确执行一次捕获：

```bash
python3 scripts/macos_ocr.py --app wechat --capture-and-ocr
python3 scripts/macos_ocr.py --app wechat --capture-and-ocr --focused-window
```

唯一窗口模式要求目标应用只有一个符合条件的在屏窗口；多个窗口返回 AMBIGUOUS_WINDOW，不选第一个/最大窗口。焦点模式是另一个显式选项，要求辅助功能信任、已有且未最小化的标准焦点窗口，并使 AX 几何与捕获列表唯一匹配。焦点模式无法匹配时不会退回唯一窗口模式。

`--focused-window` 不表示抢占系统焦点。它读取目标应用现有的焦点窗口，不通过标题猜测，也不向应用发送点击。几何只在内存中比对（0.5 point 容差）；报告至多包含匹配计数，不输出坐标、标题、PID、句柄或窗口编号。匹配位置/尺寸不等于账号或会话已验证。

## 3. 单窗口捕获边界

目标枚举只接受 qq / wechat 对应的精确 bundle。应用实例数不是 1 时不截图。ScreenCaptureKit 的 shareable-content API 会暂时返回系统可共享窗口元数据，代码只保留目标 pid/bundle、on-screen、layer 0 的候选；其他应用的像素不进入捕获。不是“从未枚举其他应用元数据”。

使用公开的 `SCContentFilter(desktopIndependentWindow:)` 和 `SCScreenshotManager.captureImage`，关闭光标、子窗口和单窗口阴影。不使用整屏裁剪、显示器过滤器、鼠标坐标或截屏命令作为 fallback。

在捕获前、图像取得后、OCR 后重新检查实例、进程启动时间、窗口 ID、窗口几何、内容尺寸、比例和权限。变化则丢弃识别摘要。窗口内切换聊天可能不改变这些字段，因此不宣称会话新鲜度已成立；消息解析和绑定仍须单独设计。

CLI 捕获分支先在主 actor 初始化自身 NSApplication 并设置 `.prohibited` 激活策略。实机早期出现 `CGS_REQUIRE_INIT` 断言退出，补上进程初始化后能正常枚举窗口。不创建窗口、不激活聊天软件。

## 4. 本地 OCR 与资源限制

固定 `VNRecognizeTextRequestRevision3`，accurate 模式，zh-Hans/en-US。先检查支持语言，关闭语言纠正和自动语言猜测，不用普通大模型修复金额、编号和正文。空图像识别结果单独为 OCR_EMPTY，不解释为没有消息或空草稿。

| 边界 | 实现 |
| --- | --- |
| 图像大小 | 最长边不超过 4096，像素数不超过 8,388,608；按窗口 pointPixelScale 计算，必要时等比缩小并标记 downscaled |
| 尺寸验证 | 非有限/非法几何拒绝；收到图像尺寸必须匹配请求计划，不符合则不 OCR |
| 输出文字 | 最多 512 个识别行、32,768 个字符，单行最多 4096；截断或缺失候选明确标为部分结果 |
| 坐标 | Vision 的左下角归一化框转成图像左上角归一化框；不输出桌面点击坐标 |
| 排序 | 按几何位置排序，不宣称识别了发言人、气泡边界、群聊提及或引用关系 |
| 置信信息 | 记录低于 0.7 的识别行数量作为诊断，不把引擎置信度当业务正确率 |
| 时间 | 内部 10 秒软预算，Python 包装器默认 15 秒硬时限，可设置 1～30 秒；超时后 kill + wait，再返回结果未知 |

首次合成空图的 Vision 调用曾耗时约 30.8 秒，后续同批明暗图在秒内完成；尚未确定冷启动原因或建立性能基准。不能承诺首次调用也在默认预算内。即使提高外层时限，内部已过期的结果仍不会被接受；未来需要单独的初始化/预热设计，而不是取消超时保护。

这些预算限制本项目的请求与导出，不保证操作系统框架内部所有分配都受同一内存上限约束。图像与原文仅在短生命周期内存中使用，不承诺同用户恶意进程、系统崩溃诊断或框架缓存的全面隔离。

## 5. 结果和失败证据

Python 包装器复用 G2a 的进程所有权、stdout 16 KiB 上限与 kill/wait 行为，并增加独立封闭 schema 校验。重复 JSON 字段、伪造目标/模式、文字扩展、矛盾计数、部分结果谎称完整、失败携带旧 OCR 摘要均被拒绝。

| 状态 | 解释 |
| --- | --- |
| METADATA_ONLY / NOT_RUNNING / PERMISSION_REQUIRED | 没有进入捕获；应用未启动和权限不足是有效负面观察，不是兼容性通过 |
| AMBIGUOUS_WINDOW / NO_ELIGIBLE_WINDOW | 未找到唯一可验证窗口，不换路径抓整屏 |
| ACCESSIBILITY_REQUIRED / NO_FOCUSED_WINDOW | 显式焦点模式的 AX 前提缺失，不改变权限或窗口 |
| TARGET_CHANGED | 前后目标不一致，丢弃 OCR；可能已截图，capture_state 如实保留 |
| CAPTURE_FAILED | 调用已开始但结果不可确认，capture_state=UNKNOWN，不宣称没有截图 |
| CAPTURE_SIZE_MISMATCH / OCR_FAILED / LANGUAGE_UNAVAILABLE | 图像或识别未满足约束，不给成功 OCR 摘要 |
| OCR_SUMMARY / OCR_PARTIAL_SUMMARY / OCR_EMPTY | 有界窗口识别统计，不等于聊天完整性、账号身份或发送能力 |

原生进程超时、异常退出或报告不可解析时，外层只要已经启动带捕获许可的进程，就保守报告 capture/ocr UNKNOWN。合法负面观察退出码 0；包装失败退出码 2。不能用 exit 0 直接标记 C03/C04 或应用 ACCEPTED。

## 6. 目前的实机事实与剩余阻塞

本轮微信 4.1.13、macOS 27.0 arm64：默认未截图；唯一窗口模式发现两个在屏候选而拒绝任意选择。显式焦点模式没有几何匹配，脱敏诊断为两个候选、位置匹配 0、尺寸匹配 0、完整匹配 0。未放宽规则猜目标，也未确认实际页面类型。

因此，本轮 **没有微信窗口截图/OCR成功的证据**，不能把合成图 OCR 成功写成微信收发可用。QQ 仍未运行，结构化样本和截图路径保持未验收；不自动启动或登录客户端。

下一项是 **G2b-2：原生窗口身份/选窗与实际捕获验收**。先用专用测试窗口或用户明确选定窗口核对 AX 与 ScreenCaptureKit 的绑定；多窗口时使用明确选择，不偷偷更换算法或只取最大窗口。完成实际捕获后才进入消息区域切分、气泡/发言人和会话身份桥接，真实发送继续保持 G3 独立门禁。

## 7. 验收范围与依据

合成 OCR 测试通过 CoreText 在内存生成中文/英文/订单编号/金额的明暗图和空图，实际调用本机 Vision；不加载用户图片。窗口策略测试使用 FakeWindowSource，不能算 ScreenCaptureKit 的真实像素捕获测试。Python 测试覆盖报告和进程边界。G1 集成脚本追加新的 Swift bundle 计数，按 bundle 汇总而不是只取最大总数，不把 XCTest 外层汇总重复计数。

本增量对应 OC-01～OC-05、MC 和 TX 相关只读/策略子集；完整案例与真实应用支持仍以独立回执的范围为准。没有复制上游实现、下载独立 OCR 权重、引入新的模型服务或改变 G1 数据库。

第一方资料为 [Apple ScreenCaptureKit](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager)、[Vision 文字识别](https://developer.apple.com/documentation/vision/recognizing-text-in-images) 和本机 Xcode SDK 中 SCShareableContent.h / SCStream.h / SCScreenshotManager.h。编译、测试及限制来自实际本地证据，不由 API 存在推导原生客户端已经适配。
