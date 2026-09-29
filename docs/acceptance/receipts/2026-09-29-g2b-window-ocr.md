# G2b-1 单窗口 / 本地 OCR 验收回执

- 日期：2026-09-29
- 分支：feat/g2b-window-ocr
- 被测完整提交：`7a4ba7bc85b1d9f905c80b6bb3bd04bab6ccfe07`
- 前置提交：`724f26d7c50502970bd6f884670946d0aa6f52ea`
- 关联：[开发说明](../../development/G2_WINDOW_OCR.md) · [矩阵](../../adapters/CAPABILITY_MATRIX.md) · [机器摘要](2026-09-29-g2b-checks.json)
- 结论：G2b-1 实现与本地集成集合通过；实际微信窗口捕获 BLOCKED，QQ 运行中适配 NOT_RUN。不是完整 G2b 或消息收发验收。

## 1. 环境与测试方式

本地 macOS 27.0 / arm64；Rust 1.98.1；Apple Swift 6.4 / Xcode 27。Swift manifest 下限 macOS 26，未在 macOS 26 真机验证。Windows、Android 不在本轮范围。

仅在内存生成合成 OCR 图片，业务回归使用 loopback HTTP 与模拟发送，不请求外部模型或读取已有 API key。受限原生探测只输出预设状态/计数，未写聊天、未发送消息、未修改系统授权；不导出联系人、正文、草稿、截图文件或坐标。

## 2. 固定提交后的集成结果

执行：`python3 scripts/g1_integration_check.py --with-macos-probe`

本地报告：`target/g1-integration/e84d3763f3d94268bdd80819285d3bdc/report.json`。该路径由 Git 忽略，机器摘要单独入库；不是下载地址。

源码指纹在检查前后均为：

```text
1f5f2108375476704eea6ba18ae6163ab4e06f52f5a97869ee74694bf4645b20
```

检查前后 HEAD 都是上面的被测提交，source_unchanged=true，11组集成检查通过。写本回执时不修改被测实现。

| 检查 | 真实结果 |
| --- | --- |
| Rustfmt / Clippy -D warnings | PASS |
| Rust 工作区回归 | 106 passed，0 failed |
| 关闭加密功能的独立过滤 | 1 passed，验证无明文降级 |
| HTTP / Host 二进制构建 | PASS |
| 两组 loopback smoke | PASS，未重复模拟发送 |
| Python 报告/监督器/集成检查 | 37 passed |
| Swift XCTest | 51 passed：ProbeKit 20 + OCRKit 31；无失败 |
| 本地文档 / Git whitespace | PASS；代码提交时18份Markdown、103处链接、12份JSON示例、61个案例编号 |

Swift 测试包含两套 XCTest bundle。集成脚本按 bundle 汇总，不仅取最大值，也不将外层重复总数累加。51 中的31是本轮新增（12项 OCR/几何/统计，19项窗口策略）；窗口源替身不算真实 SCK 像素捕获。

HTTP/宿主 smoke：宿主3次生成作业、2次模拟发送、3次回执；重放生成/发送均0。此为短时功能验证，未进行30分钟、4小时或隔夜运行。

## 3. 本地 Vision 的实际识别证据

在内存 CoreText 图片上实际运行 Vision：浅色背景和深色背景的中文、英文、订单编号、数字、小数金额均通过相应断言；空图无识别行。使用固定 zh-Hans/en-US、request revision3、关闭语言纠正。没有只比较预置字符串冒充调用 OCR。

边界回归覆盖图像尺寸/像素、非法几何、坐标原点转换、低置信、行/字符上限、缺失候选标记、不导出文本；不是完整聊天语料准确率，也没有测 P50/P95、峰值内存或移动耗电。

早期首次空图识别用时30.816秒，随后明暗图分别约0.302/0.112秒。后续重新运行更快，但未确认慢首次调用原因。保留10秒软预算和15秒默认进程硬时限；冷启动仍可能超时，不能宣称首次低延迟达标。

## 4. 真实客户端观察（独立于集成 PASS）

代码提交后再次运行：

```bash
python3 scripts/macos_probe.py --app qq --allow-ax-read
python3 scripts/macos_ocr.py --app wechat --capture-and-ocr --focused-window
```

| 项目 | 观察 | 能力结论 |
| --- | --- | --- |
| QQ | running_instances=0，NOT_RUNNING；未启动QQ | 运行中AX/编辑器/截图均NOT_RUN |
| 微信版本与权限 | 4.1.13；screen_capture_preflight=true | 仅确认前提，不是截图成功 |
| 微信焦点窗口匹配 | 2个候选，origin_matches=0、size_matches=0、frame_matches=0 | 当前没有可验证绑定；NO_ELIGIBLE_WINDOW |
| 捕获 / OCR | capture_state=NOT_ATTEMPTED、ocr_attempted=false | **微信原生捕获/OCR链路BLOCKED** |
| 账号/会话/发送 | UNVERIFIED / UNVERIFIED / NOT_IMPLEMENTED | 不生成收发授权 |

开发期间唯一窗口模式也曾报告2候选 AMBIGUOUS_WINDOW；不任意选择一个。当前未确定实际页面、窗口映射差异来源或跨Space影响。没有放宽匹配条件、抓整屏或强行激活窗口来取得绿色结果。

## 5. 修正与失败记录

初轮 Swift 编译在测试中的 `.infinity` 发生类型歧义，改为 CGFloat.infinity 后编译/测试通过。

原生捕获分支初次运行时进程 SIGABRT，脱敏诊断显示 CGS_REQUIRE_INIT。为 CLI 在主 actor 初始化自身 NSApplication（禁止激活策略）后，原生流程正常到达窗口枚举/匹配。失败报告曾为 capture/ocr UNKNOWN；不把异常退出当“肯定没有截图”。现有测试校验默认不捕获、初始化策略、失败报告不伪造成功。

一次临时诊断源码写入 target 路径被 WebCodex 策略拒绝，未绕过；一次批量精确编辑因锚点不匹配整批未执行，修正请求后完成。它们不是软件通过证据，源码与最终回归以固定提交为准。

## 6. 门禁边界及下一项

OC-01/OC-02仅覆盖有限合成文字/明暗图，不包含繁体、真实长气泡、群昵称、否定词全套；OC-03/OC-04有几何/过期/截断策略替身；OC-05只做一次性探针与有限退出，不是持续OCR调度。OC-06仍没有完整资源指标。MC-01/MC-02的真实窗口捕获未完成，MC-04的运行中QQ未完成；不把整组案例标PASS。

下一项 G2b-2：专用测试窗口或用户明确选择窗口的身份绑定、几何/Space核对与实际单窗口捕获。得到真实捕获证据后再做消息区域和发言人解析；G3真实发送仍单独验收。G1管理/存储、跨端执行、安全发布等剩余项不因本轮OCR实现而关闭。
