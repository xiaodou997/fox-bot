# FoxBot 适配能力矩阵

- 编号：FB-MATRIX-001
- 日期：2026-09-28
- 适用设计：[FB-BASELINE-001](../design/BASELINE.md)
- 状态：完整聊天适配器仍未实现/未验收；已有独立G2a macOS只读可读性探针，不能将其等同于消息收发适配。上游源码只能作为路径参考。
- 来源：[固定提交与审计记录](../references/UPSTREAM_AUDIT.md)

## 1. 状态语义

不能把“参考项目存在代码”和“FoxBot 已经支持”写进同一列。

| 层次 | 状态 | 含义 |
| --- | --- | --- |
| 上游证据 | CODE_OBSERVED | 固定提交中存在已注册路径或实现，不代表本项目复现了上游实测。 |
| 上游证据 | DISABLED | 固定提交主动禁用，不能因类名仍存在就记为支持。 |
| FoxBot 范围 | PRIMARY | 首批对齐组合；是交付目标，不是当前能力。 |
| FoxBot 范围 | PROBE_ONLY | 独立探测专项，结果可能不可用；不占首批正式支持承诺。 |
| FoxBot 范围 | DEFERRED | 后续扩展，不自动进入一期验收。 |
| 实现 | PLANNED / IMPLEMENTED | 尚未实现 / 已有代码；二者都不代表验收通过。 |
| 验证 | NOT_RUN / PASS / FAIL / BLOCKED / NA | 案例执行状态；NA 必须附理由，BLOCKED 不可算 PASS。 |
| 发布能力 | ACCEPTED | 指定提交、系统、应用版本、配置与模式下已有通过回执。其他组合不继承。 |

## 2. 首批平台组合与上游依据

| 适配器 ID | 平台 / 软件 | 范围 | 上游获取路径 | 上游写入边界 | FoxBot 当前状态 |
| --- | --- | --- | --- | --- | --- |
| AD-QQ | Android / QQ | PRIMARY | 无障碍 resource-id / text；气泡布局用于方向解析 | ACTION_SET_TEXT / 粘贴回填，不发送 | PLANNED；NOT_RUN |
| AD-X | Android / X 私信 | PRIMARY | Compose 节点 contentDescription 解析 | 通用输入框回填，不发送 | PLANNED；NOT_RUN |
| AD-FS | Android / 飞书 | PRIMARY | 无障碍气泡矩形/状态＋本地 ML Kit OCR | 通用输入框回填，不发送 | PLANNED；NOT_RUN |
| MC-WX | macOS / 微信 | PRIMARY | CaptureApp → 窗口截图与 Apple Vision 路径 | fill_text 回填，不发送 | PLANNED；NOT_RUN |
| MC-QQ | macOS / QQ | PRIMARY | AXApp → AX 文本/类属性解析 | AX 设值及输入事件降级；不发送 | PLANNED；NOT_RUN |
| WIN-WX | Windows / 微信 | PRIMARY | 上游 README 描述 WGC＋RapidOCR | app/fill.py 坐标定位＋剪贴板粘贴；不发送 | PLANNED；NOT_RUN |
| AD-WX | Android / 微信 | PROBE_ONLY | 上游主动禁用微信入口；旧适配代码仍保留 | 不可据此宣称能够采集、回填或发送 | PLANNED；NOT_RUN；自动发送关闭 |

依据为来源审计中的 S-A、S-M、S-W。对上游注释中的风险原因仅作作者报告，不将其提升为所有版本的结论。FoxBot 不直接恢复伪装服务或已禁用路径。

飞书 Android 以参考代码实际包名 com.ss.android.lark 为基础；独立 Lark 发行包、不同区域版均需单独探测，不因中文名称相近自动算同一适配。

QQ、X、飞书和微信的具体账号、私聊/群聊、语言与版本支持不能从应用名称推导；未经身份和群聊路由验收，相关自动模式保持关闭。

## 3. 后置与不隐式承诺的组合

| 平台 / 软件 | 范围 | 处理方式 |
| --- | --- | --- |
| Windows / QQ、飞书、X | DEFERRED | 不因有 Windows 原生驱动就认为已存在应用适配器。 |
| macOS / 飞书、X | DEFERRED | 需新增应用规则、证据与回归，不复用 QQ 成功状态。 |
| iOS | DEFERRED | 可规划手动导入/分享或管理入口；普通跨应用自动读取与发送不纳入等价能力承诺。 |
| Linux、其他聊天软件、网页版本 | DEFERRED | 如后续立项，新增矩阵行，不混用原生客户端的验收结论。 |
| Android 微信之外的区域版/分身应用 | PROBE_ONLY | 实例、包名、账号和权限隔离分别验证，不共享弱身份。 |

## 4. 平台驱动与应用能力

| 能力 ID | 能力 | 最小成功条件 | 失败时不能做的事 |
| --- | --- | --- | --- |
| C01 | discover | 识别已授权应用实例与可能的新消息来源 | 不能把系统通知汇总当作完整新消息。 |
| C02 | identify | 确认账号绑定、会话和目标窗口/通知动作 | 不能只凭同名联系人或旧句柄写入。 |
| C03 | read | 提供文字、顺序、来源、可见范围和不完整标记 | 不能声称已取得未显示的全部历史。 |
| C04 | route_group | 区分发言人、可靠 mention、引用和回复对象 | 不能把全部消息都压成“对方”。 |
| C05 | draft_read | 区分空草稿、已有草稿、读取失败和输入法组字 | 不能把不可读当作空白。 |
| C06 | fill | 定位编辑器并可核对写入的内容 | 不能因 API 返回成功就执行发送。 |
| C07 | send | 在独立执行校验后触发目标应用的发送行为 | 不能在公共核心统一假定 Enter 就是发送。 |
| C08 | verify | 记录提交/己方输出/送达等不同级别的效果证据 | 不能把超时当作未发出并盲目换通道重发。 |
| C09 | navigate | 发现并进入授权会话，进入后重新校验身份 | 不能抢占用户输入或把前台切换当身份确认。 |
| C10 | recover | 重启、网络异常、账号变化后恢复与对账 | 不能重放全部历史或重新发送 UNKNOWN 任务。 |
| C11 | notify_reply | 通知提供正文、会话证据和可用回复动作 | 没有实测前不能假定目标应用具备此能力。 |

### 4.1 首批能力目标

P = 计划实现并逐项验收；E = 探索项，不作为默认依赖；— = 当前不纳入。表中没有任何一项表示已完成。

| ID | C01–C03 获取与身份 | C04 群聊 | C05–C06 草稿/回填 | C07–C08 发送/验证 | C09–C10 值守/恢复 | C11 通知回复 |
| --- | --- | --- | --- | --- | --- | --- |
| AD-QQ | P | P | P | P | P | E |
| AD-X | P | E，先私信单会话 | P | P | P | E |
| AD-FS | P | P | P | P | P | E |
| MC-WX | P | P | P | P | P | — |
| MC-QQ | P | P | P | P | P | — |
| WIN-WX | P | P | P | P | P | — |
| AD-WX | E | E | E | E，默认禁止执行 | E | E |

对支持发送但不使用 GUI 编辑器的通知/API 路径，C05/C06 可在回执中标为 NA，但必须有经过验收的直接通道、身份、权限与发送验证；不能仅因 GUI 不好读就跳过检查。

C11 仅依据 Android 平台文档列为候选。尚未测试 QQ、X、飞书或微信的通知动作；锁屏、通知脱敏、分组、撤销和动作过期也须单独验收。

## 5. 各适配器的主要缺口和验收映射

下表编号对应[验收清单](../acceptance/ACCEPTANCE_CHECKLIST.md)，所有平台都还需要公共核心与自动发送案例。

| ID | 优先探测的问题 | 必须包含的专项案例 |
| --- | --- | --- |
| AD-QQ | 节点 ID 随版本变化；长气泡方向；账号/分身识别；真实发送动作 | AD-01～AD-04；GR-01～GR-03；TX-01～TX-08 |
| AD-X | contentDescription 的语言差异、日期/已读尾部、引用嵌套 | AD-01、AD-02、AD-05；TX-01～TX-08 |
| AD-FS | 自绘气泡裁剪；发言方向状态是否可靠；滚动与截图不同步 | AD-01、AD-02、AD-06；OC-01～OC-06；GR-01～GR-03；TX-01～TX-08 |
| MC-WX | 捕获窗口身份；缩放/主题/分栏；输入区域与发送按钮 | MC-01～MC-03；OC-01～OC-06；TX-01～TX-08 |
| MC-QQ | AX 树与编辑器语义；离屏节点；草稿不可读；焦点变化 | MC-01、MC-04；GR-01～GR-03；TX-01～TX-08 |
| WIN-WX | WGC 帧与窗口坐标；多屏 DPI；前台权限和真实输入位置 | WI-01～WI-03；OC-01～OC-06；TX-01～TX-08 |
| AD-WX | 不启用旧禁用路径，仅报告正常授权下可用能力与失败边界 | AD-01、AD-02、AD-07；若提出实现需重新评审范围与发送测试 |

任意 G4 多会话值守支持声明还必须覆盖 NW-01～NW-05；G5 发布声明覆盖 RL-01～RL-05。不能用 QQ 私聊通过代替飞书群聊通过，也不能把模拟器结果写成目标手机真机结果。

## 6. 发布矩阵记录格式

后续每条 ACCEPTED 记录至少包含：

```text
adapter_id:
foxbot_commit:
artifact_sha256:
os_version_and_build:
architecture_and_device:
app_version_and_package_or_bundle:
account_instance_binding: 脱敏引用
language_theme_font_scale_dpi:
chat_type: private / group
mode: assisted / auto_suggest / auto_reply
operation_scope: current_session / multi_session
channel: semantic_gui / ocr_gui / notification / official_api
capabilities: C01 ... C11 的实现状态、用例结果、限制
model_and_ocr_versions: 含字典与预后处理版本
receipt_path:
verified_at:
known_gaps:
```

运行时权限丢失或应用更新后，即使静态矩阵有历史通过记录，也要重新检查动态前提。更新适配规则、模型、字典或输入路径后，运行受影响案例并保留旧记录，不覆盖历史证据。

## 7. 实施建议与当前结论

建议先以 MC-QQ / AD-QQ 验证结构化读取与公共协议，再验证 MC-WX / WIN-WX / AD-FS 的 OCR 路径，AD-X 验证描述文本解析。该顺序是工程建议，不改变首批交付范围。

完整消息适配器仍不能声明可读、可发或可值守。现有G2a探针单独记录如下，不提升MC-WX/MC-QQ整行状态。

### G2a 探针子能力（不是应用 ACCEPTED）

| 对象 | 探针实现 | 本轮环境观察 | 未覆盖 |
| --- | --- | --- | --- |
| macOS 权限/目标检测 | IMPLEMENTED | 本机macOS27/arm64；辅助功能和录屏preflight均为true，不触发授权请求 | 其他系统/签名身份；录屏权限不等于已经截图 |
| MC-WX 可读性摘要 | IMPLEMENTED | 微信4.1.13当前focused window返回5个AX节点、0个可读StaticText；未识别消息/编辑器 | 页面类型和账号未知，不推出所有微信界面不可读；C02/C03/C05没有通过 |
| MC-QQ 可读性摘要 | IMPLEMENTED | 当前未运行，返回NOT_RUNNING；未启动QQ | 实际QQ节点/编辑器/消息路径NOT_RUN |
| 写入/发送/导航 | NOT IMPLEMENTED | 探针无相应API或宿主联接 | C06～C10不继承核心模拟结果 |

[探针实现与运行](../development/G2_MACOS_PROBE.md)说明默认只输出封闭元数据，不保存正文。以上是本轮观察摘要；对应精确代码提交和执行证据以独立回执为准，不以退出码0或complete_traversal直接标记能力通过。

### G2b-1 截图 / OCR 子能力（不提升整行适配器状态）

| 对象 | 实现 / 本轮证据 | 当前限制 |
| --- | --- | --- |
| Apple Vision 本地识别 | IMPLEMENTED；内存合成明暗图与空图实际 OCR | 中文/英文/编号/金额的有限样例，不是完整聊天数据集或性能验收 |
| 单窗口捕获管线 | IMPLEMENTED；SCK 单窗口过滤、尺寸/目标复核、策略测试 | 真实目标成功捕获仍 BLOCKED，不把假窗口测试算真实截图 |
| MC-WX 唯一窗口模式 | 实机返回 AMBIGUOUS_WINDOW，两个在屏候选；未调用捕获 | 不选择第一个/最大窗口，不退回整屏 |
| MC-WX 显式焦点模式 | 实机两个候选均无位置/尺寸/完整几何匹配，NO_ELIGIBLE_WINDOW | 当前 capture=NOT_ATTEMPTED，OCR 未运行；窗口绑定待 G2b-2 |
| MC-QQ | 复查仍 NOT_RUNNING | 未启动/登录客户端，运行中 AX 和截图证据 NOT_RUN |
| 消息 / 草稿 / 自动回复 | 尚未从窗口 OCR 建立完整适配 | 账号/会话 UNVERIFIED；不声称 C02～C10 已通过 |

[开发与运行边界](../development/G2_WINDOW_OCR.md)记录冷启动、原生初始化修复及未完成项；输出统计不是正文或发送授权。

### G2b-2 窗口身份绑定（不是应用 ACCEPTED）

| 对象 | 实现 | 本轮环境观察 | 未覆盖 |
| --- | --- | --- | --- |
| 微信进程家族绑定 | IMPLEMENTED | 根 bundle com.tencent.xinWeChat 唯一；只接受同一安装包内固定 com.tencent.flue.WeChatAppEx 子应用作为 compositor owner | 其他微信版本/安装路径、多个根实例 |
| AX ↔ ScreenCaptureKit 绑定 | IMPLEMENTED | 使用 onScreenWindowsOnly=false 枚举，再由 AX 标准焦点窗口几何唯一匹配；不按标题/最大窗口猜测 | 不保证窗口内会话未切换 |
| MC-WX 单窗口真实捕获 | IMPLEMENTED / LOCAL PASS | 微信4.1.13、macOS27/arm64：同框 root/AppEx 候选由唯一 on-screen+active 窗口消歧，最终1个焦点匹配，IMAGE_OBTAINED，3574×2280，image_saved=false | OCR未在同次真实窗口链路完成；未测macOS26 |
| MC-QQ | NOT RUN | QQ仍未运行 | 结构化读取、窗口家族、截图均待测 |

[G2b-2说明](../development/G2_WINDOW_BINDING.md)记录根因和约束。此处的 LOCAL PASS 仅指一次真实单窗口图像取得，不提升 MC-WX 整体适配状态，也不证明聊天正文、发言人或发送能力。
