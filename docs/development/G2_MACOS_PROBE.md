# G2a：macOS 只读可读性探针

- 日期：2026-09-29
- 状态：独立探针已实现；完整消息适配器尚未实现。原生读取结果与构建测试见独立回执。
- 关联：[集成对账](G1_INTEGRATION.md) · [能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)
- 实际结果：[G1集成与G2a验收回执](../acceptance/receipts/2026-09-29-g1-integration-g2a.md)，真实应用只读观察与合成测试分别记录。

后续状态：同一 Package 已新增独立 [G2b-1 截图/OCR](G2_WINDOW_OCR.md) 产品；本文仅描述原有 ProbeCLI/ProbeKit，其默认不截图、不输出正文的行为保持不变。

## 1. 本增量实际做什么

`native/macos-probe/` 是不依赖第三方包的 Swift Package，包含纯元数据分析库 ProbeKit、原生只读 ProbeCLI 和合成树 XCTest。它不链接 foxbot-host/core/http，不访问账本或 API key，不启动模型，不包含聊天输入、发送或截图实现。

目标只接受 `qq` / `wechat`，分别匹配 `com.tencent.qq` / `com.tencent.xinWeChat`。不遍历所有应用、根据显示名模糊匹配、激活窗口、打开客户端或自动请求授权。默认只查询运行实例、应用版本、辅助功能信任和录屏权限 preflight。

只有显式 `--allow-ax-read` 才读取目标应用当前的 focused window；零实例、多实例、权限不足或不可读窗口分别返回明确状态，不随意选择一个窗口。读取结束后重查应用和 focused window；变化则丢弃树摘要。窗口相同不代表会话相同，所以账号/会话始终为 UNVERIFIED。

开发工具以 macOS26为构建下限，本轮编译/运行环境为本机macOS27/arm64；尚未在26真机复现，不据此宣称产品的完整OS支持矩阵已经验收。

## 2. 构建与运行

```bash
xcrun swift test --package-path native/macos-probe --scratch-path target/macos-probe -Xswiftc -warnings-as-errors
python3 scripts/macos_probe.py --app qq
python3 scripts/macos_probe.py --app wechat
```

下面的命令会读取指定应用的窗口结构与可读性，不保存或打印原始正文：

```bash
python3 scripts/macos_probe.py --app qq --allow-ax-read
python3 scripts/macos_probe.py --app wechat --allow-ax-read
```

使用 Python 包装入口运行，默认8秒硬时限，可在1～20秒内配置。程序不会自动构建、装工具链或更改系统权限。SwiftBuild与旧native构建目录分别解析到 scratch 目录内的已知产物；同时存在不同候选时返回 AMBIGUOUS_BUILD，不根据新旧时间猜一个。可重新在单一构建目录构建后执行。

合法报告与能力不足状态返回退出码0，因为“应用未启动/权限不足”是有效探测结果，不是支持能力PASS。构建缺失、超时、无效报告或子进程错误返回2。不要按退出码0直接修改应用适配器为 ACCEPTED。

## 3. 默认脱敏与能力状态

返回字段只允许应用枚举、固定bundle、版本、临时随机snapshot_id、权限布尔、节点计数和封闭状态。没有窗口标题、联系人、聊天正文、草稿正文、坐标、PID、原生句柄、截图或路径输出；title只区分空/非空/不可读。

原生读到的 CFString 通过长度判断可读性，不把消息/草稿字符串转为 JSON。QQ 仅借鉴上游已审计的类选择器识别候选气泡/编辑器；未匹配规则不会把搜索框当输入框。微信当前没有经过验证的编辑器规则，不把任意 TextArea 当作可发送输入框。安全文本节点不展开，也不读取其value。

Python 包装器再校验封闭报告schema；新字段、任意错误文本、额外原文等均拒绝。子进程stderr不回显，stdout最多16KiB；超时/输出过大时终止并等待子进程退出，不把半截结果当成功。直接运行原生二进制只有内部软预算，不能代替包装器的硬时限。

包装器拒绝重复JSON键、权限/运行实例与结果矛盾、计数越界、WINDOW_CHANGED携带旧树、部分树却声称唯一空草稿。构建产物解析必须落在固定scratch目录内；符号链接指向目录外时拒绝执行。探针结果只用于诊断，不能作为发送授权。

| 字段/状态 | 含义 | 不代表什么 |
| --- | --- | --- |
| METADATA_ONLY / NOT_RUNNING | 未授权AX读取 / 未发现运行实例 | 不是读取失败，更不是应用不支持 |
| PERMISSION_REQUIRED | 当前进程没有对应信任 | 不自动请求或绕过权限 |
| NO_READABLE_FOCUSED_WINDOW | 当前未取得可读取的焦点窗口 | 不自动切换到别的窗口 |
| AX_SUMMARY / AX_PARTIAL_SUMMARY | 当前窗口已返回结构摘要 / 达到边界或存在错误 | 不代表取得完整聊天上下文 |
| editor_state EMPTY/NONEMPTY/UNAVAILABLE | 未截断树的唯一候选value为空/非空，或候选不可读 | 不包含输入法组字判定，不等于C05完整通过 |
| editor_state NOT_READ/AMBIGUOUS | 没找到候选 / 候选不唯一 | 不能当空草稿 |
| account/conversation UNVERIFIED | 尚未证明账号和会话身份 | 不进入发送路由 |
| write/send NOT_IMPLEMENTED | 探针没有对应实现 | 不是只在UI上藏了发送按钮 |

## 4. 有界读取

内部软预算1.5秒，单个AX对象消息超时0.12秒；最多512节点、24层、每次最多64子节点。通过 AXUIElementGetAttributeValueCount/CopyAttributeValues 截取children，而非一次拉取无限数组。QQ类列表也分页限64项。重复节点/循环、深度、节点数、子节点截断、读取错误、到期均保留部分结果标记。

“complete_traversal=true”只表示这个有界focused window树没有触发截断/错误。窗口可能本来只暴露几个容器，这不能证明正文也可读。属性为空、缺失、非字符串与权限失败不能混写为空草稿。

节点角色读取失败时不展开未知子树；用于判断保护状态的subrole调用出错时也停止该节点。部分树即使已找到一个空/非空编辑器，也将editor_state降为AMBIGUOUS，避免遗漏第二个编辑器。遍历边界并不限制第三方应用一次AX属性返回的CFString内存大小，当前还不是生产级资源隔离验收。

探针会读取AX属性值以判断可读性，但只在短生命周期内存中使用，不输出正文；这不是“从未访问任何文字”。当前没有截图、图片OCR、发言人/消息组装、稳定账号/会话ID、窗口坐标映射或宿主事件桥接。

## 5. 测试边界

Swift单测使用合成节点树：默认/缺权限/歧义实例门禁、节点/深度/孩子/时间边界、循环、受保护节点、编辑器歧义与不可读、序列化字段。不是目标聊天软件，也不是已显示的原生测试窗口。

Python单测使用临时子进程验证封闭输出、正文扩展拒绝、stderr不透传、输出上限、超时和stdout结束但进程未退出；不操作真实应用。真实QQ/微信的结果必须另列应用版本、权限和当前窗口限制。

能力矩阵MC-QQ/MC-WX整行继续 PLANNED / NOT_RUN，另列探针证据。C01只能确认应用实例子能力；C02/C03/C05尚不完整；C06～C10不从探针继承。

## 6. 后续 G2b

在不启用发送的前提下，补授权单窗口截图与 Apple Vision 本地OCR；为运行中的QQ建立可复现的消息/编辑器样本；用户确认账号会话后才讨论Observation桥接。未获权限、目标未运行或界面不匹配时，先输出真实原因，不通过伪装无障碍、输入或强制激活绕过。

## 7. 第一方依据与复用

AX读取、数组切片与对象超时按本机Xcode27 SDK的 `ApplicationServices/HIServices/AXUIElement.h` 接口/注释实现；[Apple AXUIElement 文档入口](https://developer.apple.com/documentation/applicationservices/axuielement)与[CGPreflightScreenCaptureAccess](https://developer.apple.com/documentation/coregraphics/cgpreflightscreencaptureaccess())为资料入口。SDK明确对象级timeout不会自动应用到另一个相等对象，因此每个intern后的AX对象都设置自己的超时。

QQ的候选类选择器来自[上游固定快照审计](../references/UPSTREAM_AUDIT.md)，仅复用定位经验，没有复制Python实现、图标或模型权重。探针原创Swift代码，无第三方Swift依赖；原生可读能力仍由实际环境验证，不从SDK或上游注释推导。
