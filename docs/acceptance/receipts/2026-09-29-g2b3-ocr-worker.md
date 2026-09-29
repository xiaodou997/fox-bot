# G2b-3 macOS OCR Worker / MessageSnapshot 本地验收回执

- 日期：2026-09-29
- 被测代码提交：`8ad545cd31a00f6ad97de956e1add30d5e555424`
- 分支：`feat/g2b3-ocr-worker`
- 环境：macOS 27.0 / arm64，微信 4.1.13
- 结论：**持久 worker、真实微信 ROI OCR 与脱敏 MessageSnapshot 为 LOCAL PASS；消息分组/方向尚未经过人工逐条标注，会话身份仍 UNVERIFIED。**

## 1. 固定提交集成门禁

命令：

~~~bash
python3 scripts/g1_integration_check.py --with-macos-probe
~~~

固定提交报告：

- HEAD：`8ad545cd31a00f6ad97de956e1add30d5e555424`
- source fingerprint 前后均为 `764d18691783c06b788579a69811080cf044a59696a8b5e2b35049b03fad23fa`
- Rust 工作区：106 项
- 关闭加密功能精确测试：1 项
- Python：44 项
- Swift XCTest：67 项
- format / Clippy / build / HTTP smoke / host smoke / docs / whitespace：通过
- external model requests：0
- source unchanged：true

这些数字是本地自动化检查数量，不代表完整产品验收案例全部 PASS。

## 2. 持久 worker 与预热

`foxbot-macos-ocr --worker` 使用 JSONL 请求协议。父进程 `scripts/macos_ocr_worker.py` 为每个请求提供硬超时；超时会终止整个子进程组并 wait，不把半截报告当成功。worker stderr 不转发到用户报告。

固定提交真实运行中，空白 64×64 Vision warmup 为 **30694 ms**。本轮此前还观察到过约 31.9～32.8 秒的冷启动，以及系统已热后约 0.15 秒的 warmup；因此不能把热启动数字当冷启动承诺。

## 3. 固定提交真实微信 OCR

测试过程仅把已经运行的微信切到前台，没有点击聊天、输入、粘贴或发送。随后执行：

~~~bash
python3 scripts/macos_ocr_worker.py \
  --app wechat \
  --focused-window \
  --repeat 2 \
  --warmup-timeout-seconds 60 \
  --request-timeout-seconds 15
~~~

同一 worker 连续两次请求均返回 `OCR_SUMMARY`：

| 项目 | 第 1 次 | 第 2 次 |
| --- | ---: | ---: |
| 请求耗时 | 1030 ms | 777 ms |
| OCR 行数 | 19 | 19 |
| OCR 字符数 | 206 | 206 |
| low-confidence 行 | 13 | 13 |
| 图像 | 2240×2658 | 2240×2658 |
| capture_state | IMAGE_OBTAINED | IMAGE_OBTAINED |
| window_stable | true | true |
| image_saved | false | false |
| raw_text_included | false | false |
| network_requests | 0 | 0 |

两次 `content_scope` 都是 `CHAT_REGION_HEURISTIC`，说明 Vision 只请求当前启发式聊天 ROI，而不是整窗文本。

## 4. MessageSnapshot 脱敏结果

两次真实运行的脱敏 summary 一致：

~~~text
strategy            WECHAT_HEURISTIC_V0
message_count       10
ME                  3
THEM                7
UNKNOWN             0
sender_labeled      6
used_line_count     11
complete            false
partial_reasons     HEURISTIC_REGION
conversation        UNVERIFIED
~~~

原始 OCR 正文、sender 名称、bounding boxes 和截图均没有进入命令行输出或验收文件。

`complete=false` 是刻意保留的：固定 ROI 和气泡方向仍是启发式，本轮没有人工读取真实聊天内容进行逐条 ground truth。因此不能从 `UNKNOWN=0` 推导方向识别已经 100% 正确，也不能直接将此快照授权给自动发送。

## 5. ROI 前后短时对比

同一环境在 G2b-3 开发过程中观察到：

| 路径 | OCR 行数 | 两次请求耗时 |
| --- | ---: | --- |
| 整窗 OCR 后过滤 | 约 85 | 1410 ms / 1183 ms |
| Vision ROI | 19 | 1061 ms / 829 ms |

固定提交最终请求为 1030 ms / 777 ms。上述均是单机短时样本，不是 P50/P95、CPU、内存或长稳基准。

## 6. QQ 与未覆盖项

同轮 QQ 探针仍返回 `running_instances=0 / NOT_RUNNING`，没有自动启动或登录 QQ。

本回执不覆盖：

- 微信专用测试会话的人工标注准确率；
- 动态输入区边界与不同窗口布局/主题；
- 稳定 account/conversation identity；
- QQ 运行中 AX/OCR；
- worker 接入 Rust host 后的暂停、重启、背压与资源管理；
- macOS 26、多显示器、Spaces、最小化/睡眠恢复；
- 真实草稿、写入、发送或发送结果验证。

下一阶段应优先完成稳定会话身份与人工标注对照，再将只读 MessageSnapshot 桥接到核心 Observation。真实发送继续保持关闭。
