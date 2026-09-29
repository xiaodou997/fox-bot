# FoxBot

面向微信、QQ、飞书和 X 私信的跨平台消息自动化客户端。

**当前状态：G2b-1 单窗口截图与 Apple Vision 本地 OCR 实现已落地，保留 G1 集成与 G2a AX 探针。** 合成 OCR 有实际测试；微信窗口绑定尚未通过，QQ 当前未运行。完整消息适配器、真实发送、桌面界面和 APK 尚未实现；不把合成图识别当作微信端到端可用，也不宣布完整 G1/G2 Freeze。

## 产品边界

FoxBot 负责消息获取、会话定位、排队去重、回填与发送验证；可替换的 AI 服务负责生成业务回复。支持人工辅助、自动建议和全自动回复，不绑定 Jev，不强制增加意图分类或候选排序模型。

已绑定知识库和业务约束的自定义 AI 接口可以直接作为回复服务。用户启用指定账号和会话范围后，全自动模式不要求每条消息再次人工确认；目标不明确、人工接管或发送结果未知时，暂停受影响任务。

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

## 文档入口

| 文档 | 内容 |
| --- | --- |
| [文档导航](docs/README.md) | 阅读顺序、状态语义与变更规则 |
| [设计基线](docs/design/BASELINE.md) | 产品模式、模块职责、消息与回复协议、可靠发送、技术方向和阶段门禁 |
| [适配能力矩阵](docs/adapters/CAPABILITY_MATRIX.md) | 软件与平台组合、能力拆分、上游证据、缺口和验收映射 |
| [验收清单](docs/acceptance/ACCEPTANCE_CHECKLIST.md) | 公共核心、平台、OCR、自动发送、值守与发布测试 |
| [验收回执模板](docs/acceptance/RECEIPT_TEMPLATE.md) | 测试版本、环境、操作、证据、结果和未覆盖范围 |
| [上游与技术来源审计](docs/references/UPSTREAM_AUDIT.md) | 固定提交、参考路径、许可证与复用边界 |

## 开发顺序

设计基线 → 模拟通道与回复服务闭环 → 平台能力探测 → 当前会话自动收发 → 多会话值守 → 按平台和应用版本验收发布。

当前会话自动回复不能标记为多会话值守；写入输入框不能标记为发送成功；代码可编译不能标记为真机验收通过。

## 贡献与数据边界

当前包含原创设计文档、模拟核心、HTTP 服务及持续宿主，未复制参考聊天项目实现、未引入 OCR 权重，也未发布安装包。新增系统凭据/SQLCipher 依赖记录在来源审计。Rust 依赖固定在 Cargo.lock。后续复用必须记录源仓库、固定提交、文件和许可证，保留所需通知；本项目自身许可证尚未选定，不能将参考仓库的 MIT 自动视为本项目许可证。

禁止将 API 密钥、真实聊天记录、通知凭据、未脱敏截图或设备标识提交到仓库。测试优先使用模拟窗口、合成样本与专用测试账号；真实发送必须限定在明确授权的测试会话。
