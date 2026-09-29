# G2b-2 macOS 窗口身份绑定本地验收回执

- 日期：2026-09-29
- 被测代码提交：24af466aedf94d4e365a1b39741bb1b4d2b633b9
- 分支：feat/g2b2-window-binding
- 环境：macOS 27.0 / arm64，微信 4.1.13
- 结论：**窗口身份绑定与真实单窗口 capture-only 为 PASS；真实窗口 Apple Vision OCR 仍为 BLOCKED。**

## 1. 固定提交集成门禁

命令：

~~~bash
python3 scripts/g1_integration_check.py --with-macos-probe
~~~

固定提交报告：

- HEAD：24af466aedf94d4e365a1b39741bb1b4d2b633b9
- source fingerprint 前后均为 d921db6b0320c09d2c93cba5ca9a589c60df7550c8308faec59acc68a72fa962
- Rust 工作区：106 项
- 关闭加密功能精确测试：1 项
- Python：38 项
- Swift XCTest：55 项
- format / Clippy / build / HTTP smoke / host smoke / docs / whitespace：通过
- external model requests：0
- source unchanged：true

测试数量只描述本次本地门禁，不等于 61 个产品验收案例全部 PASS，也不是长期稳定性证据。

## 2. 窗口错配根因

脱敏诊断确认微信 4.1.13 使用多进程窗口结构：

- AX 根实例为 com.tencent.xinWeChat。
- 大窗口可由同一 WeChat.app 内嵌的 com.tencent.flue.WeChatAppEx 所有。
- onScreenWindowsOnly=true 会漏掉 AX 仍能引用的 off-screen/other-Space 窗口。
- 微信前台时，同一 AX 几何可能同时出现 root 的 on-screen+active 窗口与 AppEx 的 off-screen+inactive 镜像。

最终代码只接受固定 bundle 家族，且子进程 executable 必须位于唯一根 WeChat.app 内。焦点模式先做 AX 几何完整匹配；多个同框候选只有在恰好一个同时 onScreen + active 时才允许继续。没有使用窗口标题、联系人、最大窗口、候选顺序或旧 PID 作为选择依据。

## 3. 固定提交真实单窗口捕获

测试过程仅把**已经运行的微信应用**切到前台，没有点击会话、输入、粘贴或发送。随后执行：

~~~bash
python3 scripts/macos_ocr.py --app wechat --capture-only --focused-window --timeout-seconds 12
~~~

实际脱敏结果：

| 项目 | 结果 |
| --- | --- |
| status | CAPTURE_SUMMARY |
| selection_mode | FOCUSED_WINDOW |
| candidate_count | 26 |
| origin_matches / size_matches | 2 / 2 |
| 最终 frame_matches | 1 |
| eligible_windows | 1 |
| capture_state | IMAGE_OBTAINED |
| 图像 | 3574 × 2280，未降采样 |
| window_stable | true |
| image_saved | false |
| raw_text_included | false |
| ocr_requested / ocr_attempted | false / false |
| account / conversation | UNVERIFIED / UNVERIFIED |
| send capability | NOT_IMPLEMENTED |

图片只存在于短生命周期进程内存，没有写入仓库、临时截图文件或验收附件；回执没有记录窗口 ID、PID、标题、联系人或聊天正文。

这证明的是“当前环境能将 AX 焦点窗口可靠绑定到一个 ScreenCaptureKit 单窗口并取得图像”，**不证明截图一定是聊天页，也不证明消息、发言人、草稿或会话身份已解析。**

## 4. 真实 OCR 仍未通过

同一固定代码提交再次执行真实窗口：

~~~bash
python3 scripts/macos_ocr.py --app wechat --capture-and-ocr --focused-window --timeout-seconds 15
~~~

外层进程在 15 秒预算后返回 TIMEOUT，并按 fail-closed 规则记录：

- capture_state=UNKNOWN
- ocr_state=UNKNOWN
- image_saved=false
- raw_text_included=false

此前本地合成图的首个 Vision 调用也观察到过明显冷启动，而后续调用通常更快。本回执不据此断言根因已经完全证明，也不把 capture-only 的 PASS 补写成真实 OCR 成功。G2b-3 需要通过持久 worker / 明确预热方案重新验收。

## 5. QQ 与未覆盖项

同轮只读探针显示 QQ running_instances=0 / NOT_RUNNING，因此没有运行中 QQ 窗口身份或截图证据。

本回执还不覆盖：

- macOS 26 真机；
- 多显示器、不同 Space、最小化与睡眠恢复矩阵；
- 微信窗口内部聊天区域、气泡、方向、发言人、引用和账号/会话身份；
- 用户在同一窗口切换会话时的稳定标识；
- 真实草稿读取、写入、发送和效果验证；
- 持续 OCR worker 的资源、冷启动、P50/P95 和长稳。

下一项为 **G2b-3：持久 OCR worker / 预热与聊天区域只读解析**。真实发送继续保持关闭。
