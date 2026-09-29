# G2b-2：macOS 窗口身份绑定与真实单窗口捕获

- 日期：2026-09-29
- 状态：微信 4.1.13 的当前多进程窗口绑定已实现，并取得一次真实单窗口内存捕获证据；真实窗口 OCR、聊天解析和发送仍未验收。
- 前置：[G2b-1 单窗口 OCR](G2_WINDOW_OCR.md) · [G2a AX 探针](G2_MACOS_PROBE.md)
- 关联：[能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)

## 1. G2b-1 为什么匹配失败

原实现只枚举 ScreenCaptureKit 的 on-screen 窗口，并要求窗口 owner PID/bundle 与 com.tencent.xinWeChat 根进程完全相同。实际微信 4.1.13 并不是这个结构。

本机脱敏诊断得到：

- AX 根应用仍是唯一的 com.tencent.xinWeChat。
- 微信包含嵌套 WeChatAppEx / renderer 进程；大窗口可由 com.tencent.flue.WeChatAppEx 所有。
- onScreen-only 枚举时根进程只暴露小窗口，AX 所指大窗口不在候选中。
- 全量窗口元数据可找到与 AX 几何一致的大窗口；它属于 WeChatAppEx，并可能被 ScreenCaptureKit 标记为 off-screen/other-Space。
- 当微信实际切到前台后，AX 焦点几何与 ScreenCaptureKit 窗口出现唯一完整匹配。

因此问题不是简单 Y 轴翻转或 DPI 缩放，也不能靠放宽几何容差解决。

## 2. 新的绑定规则

焦点模式现在遵循：

1. 仍以唯一 com.tencent.xinWeChat 根实例作为应用身份。
2. 从根 .app 的实际 bundle 路径建立进程家族；微信额外只允许固定 com.tencent.flue.WeChatAppEx。
3. 子应用 executable 必须位于该唯一根应用 bundle 内，单纯伪造相同 bundle id 不进入候选。
4. ScreenCaptureKit 使用 onScreenWindowsOnly=false，因为 AX 焦点窗口可能位于其他 Space 或被系统标为 off-screen。
5. UNIQUE_WINDOW 模式仍只接受 on-screen 窗口；只有显式 FOCUSED_WINDOW 模式允许离屏候选。
6. AX 标准焦点窗口先按几何完整匹配。若存在多个同框候选，只在其中恰好一个同时为 ScreenCaptureKit 的 on-screen + active 窗口时选择它；否则继续拒绝为歧义。不会按候选顺序、PID、新旧时间或大小排序。
7. 捕获前再次验证根进程、子进程 bundle/路径、窗口 ID/owner/几何/内容尺寸/scale；捕获后仍执行稳定性复核。

没有引入标题匹配、联系人字符串、最大窗口启发式、旧 PID 白名单或整屏 fallback。

## 3. capture-only 验证入口

G2b-1 的一次性 Vision 进程存在明显冷启动，本机真实链路曾超过 30 秒。为了把窗口绑定/截图与 OCR 启动拆开验收，新增：

~~~bash
python3 scripts/macos_ocr.py --app wechat --capture-only --focused-window
~~~

capture-only 仍执行同一目标绑定、ScreenCaptureKit 单窗口捕获和捕获后稳定性复核，但不创建 Vision 请求。成功状态 CAPTURE_SUMMARY 必须满足：

- capture_state=IMAGE_OBTAINED
- eligible_windows=1
- window_stable=true
- ocr_requested=false
- ocr_attempted=false
- 有界图像尺寸存在
- image_saved=false、raw_text_included=false

它不是 OCR 成功，也不是聊天消息读取成功。

## 4. 本轮真实证据

环境：macOS 27.0 / arm64，微信 4.1.13，辅助功能和录屏 preflight 已存在。

为验证当前窗口，测试过程仅将已经运行的微信应用切到前台，没有点击会话、输入或发送。随后 capture-only --focused-window 返回：

~~~json
{
  "status": "CAPTURE_SUMMARY",
  "eligible_windows": 1,
  "capture_state": "IMAGE_OBTAINED",
  "window_stable": true,
  "image": {"width": 3574, "height": 2280, "downscaled": false},
  "ocr_requested": false,
  "ocr_attempted": false,
  "image_saved": false,
  "raw_text_included": false
}
~~~

报告没有窗口标题、窗口 ID、PID、联系人或正文。图像只存在于进程内存，没有写文件。

固定提交前的复测还出现了两个完全同几何候选：当前 root 窗口为 on-screen + active，AppEx 镜像为 off-screen + inactive。新增的二次消歧只接受前者；合成测试同时验证“两个同框但没有唯一 active 窗口”仍返回歧义。最终真实报告中 origin_matches=2、size_matches=2，而消歧后的 frame_matches=1。

同一阶段的真实 capture-and-ocr 一次性进程仍可在 Vision 阶段超过外层 30 秒而被终止；该情况保守记录 capture/OCR UNKNOWN，不拿 capture-only 的成功替它补成 OCR 成功。

## 5. 测试与未覆盖范围

新增合成回归覆盖：显式焦点模式允许经过验证的 off-screen compositor 窗口；唯一窗口模式仍拒绝纯离屏候选；capture-only 不调用 OCR；目标变化和多窗口等旧边界继续保留。

当前尚未解决：

- 一次性真实窗口 Vision 冷启动/预热和长期 worker 生命周期。
- 窗口内部从整窗文字到聊天区域、气泡、方向、发言人和引用的解析。
- 用户在同一窗口内切换会话时的稳定会话身份。
- QQ 运行中样本。
- macOS 26 真机、不同 Space/多显示器/最小化矩阵。

因此下一步是 **G2b-3：持久 OCR worker / 预热与聊天区域只读解析**。真实写入和发送继续留在 G3。
