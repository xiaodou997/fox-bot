# G2b-3：持久 Vision OCR Worker 与只读 MessageSnapshot

- 日期：2026-09-29
- 状态：worker、预热、微信聊天 ROI 与只读 MessageSnapshot 已实现；真实微信 4.1.13 持久 worker OCR 已通过本地脱敏验证。会话身份、人工标注准确率、QQ 与发送仍未验收。
- 前置：[G2b-2 窗口身份绑定](G2_WINDOW_BINDING.md) · [G2b-1 本地 OCR](G2_WINDOW_OCR.md)
- 关联：[能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [来源审计](../references/UPSTREAM_AUDIT.md)

## 1. 为什么要持久 worker

G2b-2 的一次性进程在真实微信窗口上可以稳定完成单窗口捕获，但首次 Apple Vision 文本识别曾超过 30 秒。该成本不能通过扩大每条消息的超时来解决，否则新消息链路会被首次模型加载阻塞。

G2b-3 将 `foxbot-macos-ocr` 增加 `--worker` 模式：

~~~text
父进程
  ├─ 启动独立 OCR worker
  ├─ warmup：64×64 空白内存图，不读取聊天
  ├─ capture_ocr #1
  ├─ capture_ocr #2
  └─ shutdown
~~~

worker 使用一行一个 JSON 的有界协议，最多 4096 字节请求、16384 字节响应。request id 只允许字母数字、下划线和连字符。`capture_ocr` 在成功 warmup 前返回 `NOT_WARMED`。

`scripts/macos_ocr_worker.py` 持有子进程组；每个请求有独立硬超时，超时后 kill 整个 worker 并 wait。stderr 不转发到用户报告。当前是开发/探针 supervisor，尚未接入 Rust host 常驻生命周期。

## 2. 预热事实

`VisionOCR.warmup()` 只在内存创建 64×64 白图，并运行与真实路径相同的本地 Vision request；不截图、不联网、不读取聊天。

本轮观察到两类情况：

- 真正冷启动的一次 worker：warmup 约 31.9～32.8 秒。
- Vision 已被系统/前序测试热起来后，新 worker warmup 可降至约 0.15 秒。

因此不能把 0.15 秒写成冷启动承诺。产品化时应在用户启用 macOS 读取能力后异步提前启动/预热 worker，并继续保留父进程硬超时与重启能力。

## 3. 微信 Vision ROI

G2b-2 对整窗进行 OCR，再在解析阶段过滤聊天区。G2b-3 将同一启发式区域直接设为 Vision `regionOfInterest`：

~~~text
top-left normalized:
x = 0.32 ... 1.00
y = 0.10 ... 0.76
~~~

Vision 使用 bottom-left ROI，因此内部进行坐标转换；返回 observation 再映射回完整窗口的 top-left 归一化坐标。合成测试验证 ROI 外文本被排除、ROI 内 box 仍映射到完整窗口坐标。

这组阈值借鉴上游 MIT `perception.py` 的微信 4.x 实测布局经验，来源与本地改写记录在[来源审计](../references/UPSTREAM_AUDIT.md)。它仍是启发式，不是动态输入区边界。

真实微信同一窗口对比：

| 路径 | OCR 行数 | 两次请求耗时 |
| --- | ---: | --- |
| 整窗 OCR 后过滤 | 约 85 | 1410 ms / 1183 ms |
| Vision ROI | 19 | 1061 ms / 829 ms |

两组均是单机短时观察，不是 P50/P95 性能基准。

## 4. MessageSnapshot

`MessageParser.swift` 产生只存在进程内存的：

~~~text
MessageSnapshot
  messages[]
    text            raw OCR text, non-Codable
    direction       ME / THEM / UNKNOWN
    sender?         optional group sender, non-Codable
    confidence
    bounds
    folded lines[]
  region
  partialReasons[]
~~~

诊断输出只暴露 `MessageSnapshotSummary`：

- message / me / them / unknown 计数
- sender_labeled_count
- used_line_count
- strategy
- partial reasons

不输出正文、sender 名、box 或桌面坐标。

当前策略 `WECHAT_HEURISTIC_V0` 使用保守左右阈值；中央或跨越边界的文本保持 UNKNOWN。相邻同侧、紧密且左边缘对齐的多行才折叠。较小且紧邻下一条 THEM 消息的短行可暂作 sender header，但 sender 名不会进入 JSON。

因为聊天区域尚未动态校准，所有真实 summary 都包含 `HEURISTIC_REGION`，所以 `complete=false`。这意味着 MessageSnapshot **不能直接作为自动发送授权**。

## 5. 真实微信脱敏结果

环境：macOS 27.0 / arm64，微信 4.1.13。测试仅将已经运行的微信切到前台，不点击聊天、不输入、不发送。

同一 worker 预热后连续两次 `capture_ocr` 均返回：

~~~text
status              OCR_SUMMARY
capture_state       IMAGE_OBTAINED
window_stable       true
image_saved         false
raw_text_included   false
network_requests    0
OCR lines           19
MessageSnapshot     10 messages
  ME                3
  THEM              7
  UNKNOWN           0
  sender_labeled    6
complete            false (HEURISTIC_REGION)
conversation        UNVERIFIED
~~~

ROI 后两次请求约 1061 ms / 829 ms；两次 MessageSnapshot summary 一致。该一致性只证明当前画面和启发式输出稳定，**没有人工读取真实消息逐条标注，因此不能宣称方向/分组准确率已经通过**。

## 6. 下一阶段

在桥接核心 Observation 之前仍需：

1. 给微信建立专用测试会话和人工标注数据，核对气泡分组、方向、群 sender、数字/金额和引用。
2. 动态识别输入区域上边界或建立可失效的校准，替换固定 y=0.76。
3. 设计稳定 account/conversation identity；窗口几何不等于会话身份。
4. 启动 QQ，取得运行中 AX/窗口证据。
5. 将 worker 生命周期接入 Rust host，并验证暂停、崩溃、重启和背压。

真实写入和发送继续留在 G3；G2b-3 不增加任何输入、点击或发送 API。
