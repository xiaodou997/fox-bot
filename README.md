# FoxBot

面向微信、QQ、飞书和 X 私信的跨平台消息自动化客户端。

**当前状态：G3c-1 首次真实单条发送已执行并观察到消息；回执采集缺陷已修复，自动闭环仍待收口。** 新增固定测试回复的加密发件箱、原生回填、单次发送与新己方消息核对；尚未接入真实模型或持续 AUTO_REPLY。桌面界面和 APK 未实现。入口与边界见[G3c-1 说明](docs/development/G3C1_SINGLE_REAL_SEND.md)，本轮状态见[真实发送与回执修复记录](docs/acceptance/receipts/2026-10-01-g3c1-real-attempt-and-receipt-fix.md)，历史证据见[文档导航](docs/README.md)。

## 产品边界

FoxBot 负责消息获取、会话定位、排队去重、回填与发送验证；可替换的 AI 服务负责生成业务回复。支持人工辅助、自动建议和全自动回复，不绑定 Jev，不强制增加意图分类或候选排序模型。

已绑定知识库和业务约束的自定义 AI 接口可以直接作为回复服务。用户启用指定账号和会话范围后，全自动模式不要求每条消息再次人工确认。实际运行由 FoxBot 独占聊天界面，不支持人机同时编辑，也不要求检测 IME 或键鼠活动；开发期间手动操作前先暂停/停止，结束后显式恢复。目标不明确、显式暂停或发送结果未知时，停止受影响任务。

优先读取系统暴露的结构化内容，缺失部分使用本地 OCR。Android 通过签名 APK 分发，不以 Google Play 上架为前提。来源和平台边界见[上游审计](docs/references/UPSTREAM_AUDIT.md)。

## 首批适配范围

| 平台 | 首批对齐的软件 | FoxBot 状态 |
| --- | --- | --- |
| Android | QQ、X 私信、飞书 | PLANNED |
| macOS | 微信、QQ | PLANNED |
| Windows | 微信 | PLANNED |

Android 微信列为单独的能力验证专项，当前不计入首批可交付组合。其他平台与软件的交叉组合、iOS 辅助或管理入口均不自动计入首批范围。详细边界见[适配能力矩阵](docs/adapters/CAPABILITY_MATRIX.md)。

## 本地开发与模拟演示

```bash
cargo run --locked -p foxbot-sim -- demo .foxbot-sim/g1a
cargo run --locked -p foxbot-sim -- demo .foxbot-sim/g1a
cargo test --workspace --all-targets --locked
```

使用全新状态目录时，第一次处理合成新消息并模拟发送一次；第二次不重复请求或发送。整个演示不连接模型、不读取聊天软件。该 G1a 演示仍使用未加密的合成账本，不用于真实聊天；G1c 的加密入口独立启用，不自动转换旧目录。

详细环境、命令、状态机、测试映射和未完成项见 [G1a 开发说明](docs/development/G1_RUNTIME.md)。

## HTTP 回复与回执补偿

```bash
cargo build --locked -p foxbot-http
python3 scripts/http_smoke.py
```

该 smoke 自行启动 loopback 测试服务，验证真实 HTTP 请求、模拟发送、重放去重和回执 503 后的独立补偿。只使用合成数据，不读取真实密钥或连接外部模型。配置、API 生命周期、服务端幂等/暂存契约及限制见 [G1b 开发说明](docs/development/G1_HTTP_PROVIDER.md)。

## 持续宿主与安全配置

```bash
cargo build --locked -p foxbot-host
python3 scripts/host_smoke.py
```

宿主启动时暂停，经明确 resume 后处理已配置的新消息；HTTP 期间可暂停或停止，回执补偿不重复发送。此 smoke 仅使用合成内容与本机服务，不读现有凭据。macOS 钥匙串引用和 SQLCipher 加密账本的显式入口、权限/恢复边界见 [G1c 开发说明](docs/development/G1_HOST_SECURITY.md)。加密能力不意味着真实客户端或长期值守已验收。

## 集成复核与 macOS 只读探针

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
python3 scripts/macos_probe.py --app qq
python3 scripts/macos_probe.py --app wechat --allow-ax-read
```

第一条运行本地测试并构建探针，不读取真实聊天；后两条探测已运行应用，微信命令显式允许读取AX可读性但不输出正文。探针不截屏、不发消息、不改权限，也不自动启动客户端。参见 [G1集成对账](docs/development/G1_INTEGRATION.md)与[G2a探针说明](docs/development/G2_MACOS_PROBE.md)。

## 单窗口本地 OCR

```bash
python3 scripts/macos_ocr.py --app wechat
python3 scripts/macos_ocr.py --app wechat --capture-and-ocr --focused-window
```

第一条不截图；第二条显式绑定既有焦点窗口，只有权限、几何与唯一性均满足时才调用单窗口捕获和本地 OCR。输出只含统计，不保存图像或正文。多窗口歧义或 AX/SCK 几何不匹配时停止，不退回整屏截图。当前实机阻塞、合成测试及预算见 [G2b-1 开发说明](docs/development/G2_WINDOW_OCR.md)。

## G2b-2 窗口身份绑定

~~~bash
python3 scripts/macos_ocr.py --app wechat --capture-only --focused-window
~~~

微信 4.x 的 AX 根进程与实际大窗口 compositor 进程不同，而且主窗口可能不在 onScreen-only 列表中。G2b-2 只接受固定应用家族、安装包内子进程和 AX 几何唯一匹配；不靠标题、最大窗口或 PID 猜测。真实微信 4.1.13 已取得一次 CAPTURE_SUMMARY，图片只在内存中出现。实现与边界见 [G2b-2说明](docs/development/G2_WINDOW_BINDING.md)。

## G2b-3 持久 OCR worker 与只读消息快照

~~~bash
python3 scripts/macos_ocr_worker.py \
  --app wechat \
  --focused-window \
  --repeat 2
~~~

worker 在独立进程内先用 64×64 空白图预热 Apple Vision，再复用同一进程处理真实窗口。微信 OCR 只请求启发式聊天 ROI，不再识别整个侧栏与输入区；真实测试从约 85 行整窗 OCR 降到 19 行区域 OCR，两次请求约 1.06s / 0.83s。原始 OCR 文字和 sender 仅存在进程内存，命令行只输出 MessageSnapshot 计数与不确定性。详情见 [G2b-3说明](docs/development/G2_OCR_WORKER_MESSAGE_SNAPSHOT.md)。

## G2c 会话身份与 Rust Host

    target/debug/foxbot-host native-read-probe target/macos-probe/debug/foxbot-macos-ocr --allow-native-read
    python3 scripts/g2c_ground_truth.py target/g2c-groundtruth/<private-fixture>.json

G2c 将应用运行会话指纹、会话标题指纹、用户显式 Binding、identity_epoch 和跨帧消息连续性组合使用；标题哈希或窗口几何都不能单独成为稳定会话身份。ground-truth 原始标注只能放在 Git 忽略的 target/g2c-groundtruth/，输出只含误差计数。真实标注尚未执行，因此当前真实微信只读探针仍不会生成可进入自动回复链的 Observation。详见 [G2c说明](docs/development/G2C_IDENTITY_HOST.md)。

## G2d Observation Bridge

```bash
cargo run --locked -p foxbot-host -- \
  bridge-sim-probe target/g2d-smoke \
  --allow-plaintext-synthetic
```

该入口只使用合成快照和本地 SQLite Runtime：首帧只 baseline，下一帧只把可信新增 incoming 排队，重放为 NO_CHANGE；不会启动模型、HTTP 或聊天原生写入。真实微信 Observation 验收仍要求专用测试会话的人工 ground-truth 和显式 Binding。详见 [G2d说明](docs/development/G2D_OBSERVATION_BRIDGE.md)。

## G2d 真实验收工作流

~~~bash
cargo run --quiet --locked -p foxbot-host -- \
  g2d-private-capture target/macos-probe/debug/foxbot-macos-ocr \
  g2d-test private-basic private --allow-private-test-data

python3 scripts/g2d_ground_truth.py g2d-test
~~~

真实正文和人工标注只保存在 Git 忽略的 `target/g2d-real/<session>/`；采样会把 observed 写入私有文件，但 expected 永远默认为空，防止 OCR 自我验收。完成 6 类人工标注并通过 acceptance 后，才能建立 baseline，再由另一测试账号发送一条已知 incoming，最后执行 real verify。完整步骤见 [G2d 真实验收说明](docs/development/G2D_REAL_ACCEPTANCE.md)。

人工确认推荐使用本机交互式审核器，而不是手改 JSON：

~~~bash
python3 scripts/g2d_review_ground_truth.py g2d-test
~~~

审核器只允许在真实 TTY 中显示私有正文；被管道、自动化或远程日志调用时会直接拒绝，因此聊天内容不会被带入普通验收日志。

推荐通过状态引导器推进：

~~~bash
python3 scripts/g2d_real_status.py init g2d-test
python3 scripts/g2d_real_status.py status g2d-test
~~~

它只输出当前阶段、缺失场景和下一条命令，不读取或输出真实聊天正文。

## 文档入口

| 文档 | 内容 |
| --- | --- |
| [文档导航](docs/README.md) | 阅读顺序、状态语义与变更规则 |
| [设计基线](docs/design/BASELINE.md) | 产品模式、模块职责、消息与回复协议、可靠发送、技术方向和阶段门禁 |
| [适配能力矩阵](docs/adapters/CAPABILITY_MATRIX.md) | 软件与平台组合、能力拆分、上游证据、缺口和验收映射 |
| [验收清单](docs/acceptance/ACCEPTANCE_CHECKLIST.md) | 公共核心、平台、OCR、自动发送、值守与发布测试 |
| [验收回执模板](docs/acceptance/RECEIPT_TEMPLATE.md) | 测试版本、环境、操作、证据、结果和未覆盖范围 |
| [上游与技术来源审计](docs/references/UPSTREAM_AUDIT.md) | 固定提交、参考路径、许可证与复用边界 |
| [G2c 会话身份与 Rust Host](docs/development/G2C_IDENTITY_HOST.md) | 稳定会话身份、ground-truth 门禁和 Rust 原生读取宿主 |
| [G2d Observation Bridge](docs/development/G2D_OBSERVATION_BRIDGE.md) | 私有快照到 Runtime ingest、幂等恢复与真实验收边界 |
| [G2d 真实验收工作流](docs/development/G2D_REAL_ACCEPTANCE.md) | 私有采样、人工标注、显式 Binding、baseline 与单条真实 incoming 验收 |

## 开发顺序

设计基线 → 模拟通道与回复服务闭环 → 平台能力探测 → 当前会话自动收发 → 多会话值守 → 按平台和应用版本验收发布。

当前会话自动回复不能标记为多会话值守；写入输入框不能标记为发送成功；代码可编译不能标记为真机验收通过。

## 贡献与数据边界

当前包含原创设计文档、模拟核心、HTTP 服务及持续宿主，未复制参考聊天项目实现、未引入 OCR 权重，也未发布安装包。新增系统凭据/SQLCipher 依赖记录在来源审计。Rust 依赖固定在 Cargo.lock。后续复用必须记录源仓库、固定提交、文件和许可证，保留所需通知；本项目自身许可证尚未选定，不能将参考仓库的 MIT 自动视为本项目许可证。

禁止将 API 密钥、真实聊天记录、通知凭据、未脱敏截图或设备标识提交到仓库。测试优先使用模拟窗口、合成样本与专用测试账号；真实发送必须限定在明确授权的测试会话。
