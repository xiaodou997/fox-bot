# FoxBot 适配能力矩阵

- 编号：FB-MATRIX-001
- 日期：2026-10-01
- 适用设计：[FB-BASELINE-001](../design/BASELINE.md)
- 状态：macOS 微信 current-session 的 G2 读取已真实 Freeze，G3a 已取得测试级真实回填/回读；G3b 已改为无人值守独占契约，IME/人工活动不再阻断开发。G3c-1 已实现固定短文本 C07/C08，真实验收单独记录；完整自动回复、导航与恢复另行验收。
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
| MC-WX | macOS / 微信 | PRIMARY | CaptureApp → 窗口截图与 Apple Vision 路径 | CGEvent Unicode 测试回填，不发送 | G2 READ FREEZE；G3a C06 REAL PASS；G3b UNATTENDED；G3c-1 C07/C08 IMPLEMENTED；C09/C10 NOT_RUN |
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
| C05 | draft_read | 区分空草稿、残留草稿和读取失败；不要求 IME 证明 | 不能把不可读当作空白。 |
| C06 | fill | 定位编辑器并可核对写入的内容 | 不能因 API 返回成功就执行发送。 |
| C07 | send | 在独立执行校验后触发目标应用的发送行为 | 不能在公共核心统一假定 Enter 就是发送。 |
| C08 | verify | 记录提交/己方输出/送达等不同级别的效果证据 | 不能把超时当作未发出并盲目换通道重发。 |
| C09 | navigate | 发现并进入授权会话，进入后重新校验身份 | 显式暂停时不能导航；不能把前台切换当身份确认。 |
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

### G2b-3 持久 OCR / MessageSnapshot（不是应用 ACCEPTED）

| 对象 | 实现 | 本轮环境观察 | 未覆盖 |
| --- | --- | --- | --- |
| Vision 持久 worker | IMPLEMENTED / LOCAL PASS | JSONL 子进程显式 warmup；请求超时由父进程 kill+wait；同一 worker 连续两次真实 OCR 成功 | 尚未接 Rust host 生命周期；冷启动仍可能约 32.8s |
| 微信聊天 ROI | IMPLEMENTED / LOCAL PASS | 只对 top-left 归一化 x≥0.32、y 0.10～0.76 的启发式聊天区跑 Vision；真实 OCR 行数约 85→19 | 输入区高度/主题/窗口布局仍未动态校准 |
| MC-WX 真实 OCR | IMPLEMENTED / LOCAL PASS | 微信4.1.13、macOS27/arm64；连续两次 OCR_SUMMARY，请求约1061ms/829ms；不保存图像/正文 | 未测macOS26、多显示器/Space/最小化；未做标注准确率 |
| MessageSnapshot | IMPLEMENTED / HEURISTIC | 实际脱敏 summary 两次均为10条：3 ME / 7 THEM / 0 UNKNOWN，sender_labeled=6 | 只证明启发式输出稳定，不证明10条均为真实聊天气泡；conversation仍UNVERIFIED |
| MC-QQ | NOT RUN | QQ仍未运行 | 运行中AX、OCR或专属 MessageSnapshot 未验收 |

[G2b-3说明](../development/G2_OCR_WORKER_MESSAGE_SNAPSHOT.md)记录协议、ROI 和真实性能证据。HEURISTIC_REGION 会强制 summary.complete=false，因此这些结果不能直接成为自动发送授权。

### G2c 身份 / Ground Truth / Host 桥接（真实发送仍关闭）

| 对象 | 实现 | 本轮状态 | 未覆盖 |
| --- | --- | --- | --- |
| 应用运行会话 | IMPLEMENTED | bundle + launch time 本地 SHA-256；进程重启后旧 binding 变 PROVISIONAL | 同进程内登出/换号尚无自动系统信号，需显式 invalidate/rebind |
| 会话身份 | IMPLEMENTED | 标题仅作 SHA-256 视觉指纹；必须用户显式映射到稳定 account/conversation Binding；同指纹重复配置拒绝 | 同名会话仍依赖至少两条消息连续性和显式绑定，不能把标题当原生 ID |
| 跨帧消息跟踪 | IMPLEMENTED | 首帧 historical baseline；≥2 条 suffix/prefix 连续后只生成新增 Observation；重复“好的”可作为独立消息 | 滚动跨度过大/无重叠返回 AMBIGUOUS，不猜测 |
| Ground truth | REAL PASS | 私有标注文件仅允许在 target/g2d-real；最终 6 canonical case / 60 条 expected，覆盖 private/group/duplicate/numeric/multiline/reference；direction/sender/count error=0，CER≈0.08% | 仅针对本次测试环境和已冻结 parser/规则 |
| Rust worker host | IMPLEMENTED / LOCAL PASS | starts paused；resume warmup；pause 结束 worker；崩溃后下一读重启预热；有界队列返回 backpressure；reader 线程受管 | 尚未并入长期生产调度 tick |
| MC-WX Rust 只读探针 | REAL PASS | 真实 acceptance=true；显式 Binding + baseline 后另一测试账号新增 1 条 incoming，Runtime NEW / queued=1；重复读取 NO_CHANGE；公开输出无正文/哈希值 | 仅 current-session read；写入/发送仍关闭 |

[G2c说明](../development/G2C_IDENTITY_HOST.md)记录门禁细节。G2c 没有新增任何原生写入/点击/发送能力。

### G2d Observation Bridge（真实发送仍关闭）

| 对象 | 实现 | 本轮状态 | 未覆盖 |
| --- | --- | --- | --- |
| Snapshot → Runtime | IMPLEMENTED / REAL PASS | 真实 baseline 11 条；当前帧唯一新增 1 条 queued；重放 NO_CHANGE；runtime messages=12，tasks/ready/unresolved_send=0 | 不代表 draft/fill/send 已实现 |
| 游标提交 | IMPLEMENTED / PASS | Runtime 全部接收后才提交 bridge cursor；失败时可重试相同 canonical IDs | 跨进程持久化 bridge cursor 尚未设计 |
| Backpressure 恢复 | IMPLEMENTED / PASS | 中途失败后已写 prefix 在重试中为 Duplicate，后缀不丢失 | 生产背压策略仍由宿主调度决定 |
| PROVISIONAL / AMBIGUOUS | IMPLEMENTED / PASS | 不调用 Runtime::ingest，不产生消息 | 无 |
| MC-WX 真实 Observation | REAL PASS / G2 FREEZE | 6 case / 60 条 accepted GT；正确 private baseline；另一账号真实 incoming 触发 NEW / queued=1；repeat NO_CHANGE | 仅当前会话只读链；G3 写入和发送独立验收 |

[G2d说明](../development/G2D_OBSERVATION_BRIDGE.md)和[回执](../acceptance/receipts/2026-09-29-g2d-observation-bridge.md)记录固定提交证据。G2d 没有调用 AI，也没有增加聊天写入/发送能力。

### G2d 真实验收 Harness

| 对象 | 实现 | 本轮状态 | 未覆盖 |
| --- | --- | --- | --- |
| 私有 case 采样 | IMPLEMENTED / REAL PASS | 连续两读稳定才写 `target/g2d-real/<session>`；最终 6 个 canonical case 完成 | 私有正文仍只保留本机 ignored 目录 |
| Ground-truth evaluator | IMPLEMENTED / REAL PASS | 6 case / 60 条；direction/sender/count error=0；字符级 CER≈0.08%；acceptance=true | 只对冻结测试样本和环境成立 |
| Baseline preflight | IMPLEMENTED / REAL PASS | accepted=false 会在 worker 前拒绝；accepted=true 后 private baseline 额外拒绝 group sender evidence / unresolved identity | baseline 不是发送授权 |
| Real verify Runtime | IMPLEMENTED / REAL PASS | 随机 SQLCipher Runtime；真实 baseline 11 条；单条 incoming queued=1；重读 NO_CHANGE；key zeroize、临时目录删除 | 未调用 ReplyProvider / fill / send |
| MC-WX readiness | SUPERSEDED BY G2 FREEZE | 早期 readiness 仍保留为历史回执；最终真实 Observation 已 PASS | 见 2026-09-30 G2 Freeze 回执 |

[真实验收工作流](../development/G2D_REAL_ACCEPTANCE.md)、[readiness 回执](../acceptance/receipts/2026-09-29-g2d-real-readiness.md)和[G2 Freeze 回执](../acceptance/receipts/2026-09-30-g2-freeze.md)记录完整证据。G2 真实读取链已 Freeze；C05 draft_read / C06 fill 从 G3a 起单独验收。

### G3a Draft Writer（测试级真实回填；发送仍关闭）

| 对象 | 实现 | 本轮状态 | 未覆盖 / 限制 |
| --- | --- | --- | --- |
| 微信 AX 编辑器语义 | PROBED / NOT AVAILABLE | 当前微信 4.1.13 focused window 只暴露约 5 个 AX 节点；无 AXTextArea / AXTextField、无 settable AXValue、点击输入区后也没有 AXFocusedUIElement | 不能用 AXValue 精确读取/写草稿 |
| C05 draft_read | IMPLEMENTED / HEURISTIC | 输入区局部 Vision 可识别可见草稿；已知空输入占位文案会排除；OCR partial 记 UNREADABLE；真实已有草稿返回 NONEMPTY 并拒绝覆盖 | 隐藏/不可见草稿与 OCR 漏识别仍需按实际场景验证；不标 C05 ACCEPTED，IME 不再是验收项 |
| C06 fill | IMPLEMENTED / REAL PASS / TEST ONLY | 独立 `foxbot-macos-draft`：要求微信系统前台、唯一 focused window、exact conversation fingerprint、显式 `--allow-heuristic-empty-test`；空草稿时写入唯一测试文本并 OCR 回读完全匹配 | 生产自动模式仍待 G3c 接入；布局/主题和支持文本需验收，不再要求人工并发输入 |
| 已有草稿保护 | REAL PASS | 真实 NONEMPTY 草稿下返回 `DRAFT_NOT_EMPTY_OR_UNREADABLE`，`write_attempted=false`，`send_attempted=false` | 只证明可见 OCR 草稿 |
| 发送动作 | NOT IMPLEMENTED | report 固定 `send_attempted=false`；代码无 Enter/点击发送路径 | C07/C08 全部留给 G3b/G3c |

[G3a 说明](../development/G3A_DRAFT_WRITER.md)和[真实回执](../acceptance/receipts/2026-10-01-g3a-draft-writer.md)记录本轮真机证据。G3a PASS 只表示受限测试条件下可安全回填并回读，不提升 AUTO_REPLY。

### G3b Safe Send Gate（本阶段只做判定；发送见后续 G3c-1）

| 对象 | 实现 | 本轮状态 | 未覆盖 / 限制 |
| --- | --- | --- | --- |
| Runtime BEFORE_FILL | IMPLEMENTED / PASS | 无副作用 preview 检查 host pause、action/revision/profile、attempt budget、prior UNKNOWN/SUBMITTED、target/identity、app-session、conversation surface、frontmost、conversation_changed、permission、draft empty；不要求 IME/人工活动 | 需要具体平台提供可信 live facts |
| Runtime BEFORE_SEND | IMPLEMENTED / PASS | fill 后必须同 app-session / conversation surface / window / editor / layout，且 draft exact-match outbound text；失败时 dispatch 进入 UNKNOWN 且不调用 send | 本阶段不触发真实 send |
| Gate-only smoke | PASS | 两阶段均 allowed；action 仍 PREPARED；fill_calls=0、send_calls=0、outgoing=0 | synthetic only |
| Host GUI ownership | PASS | queued incoming / manual own output / stop / DeviceOwner inode replacement 均在 fill/send 前阻断 | 多设备全局排他仍不在一期内 |
| MC-WX native facts | IMPLEMENTED / 新版真机 NOT_RUN | v2 按独占契约检查七项执行事实；不采集 IME/键鼠活动；旧版真机其它事实通过的记录保留 | 旧回执不自动转为新版 PASS；write/send ops 仍为 0 |
| C07 send | G3b 阶段不实现 | G3b 仅检查；G3c-1 另有原生发送 worker | 不再等待 IME；见下方 G3c-1 |

当前规则见[G3b 说明](../development/G3B_SAFE_SEND_GATE.md)和[无人值守契约](../development/G3B_UNATTENDED_EXECUTION.md)。[旧回执](../acceptance/receipts/2026-10-01-g3b-safe-send-gate.md)保留历史结果；移除不适用条件不等于将旧真机 BLOCKED 改成 PASS。

### G3c-1 单条当前私聊发送

| 对象 | 实现 | 当前边界 |
| --- | --- | --- |
| NativeSendChannel | IMPLEMENTED | 复用 Runtime 双门禁、DeviceOwner 和加密 outbox；单次写入/发送，不接真实模型 |
| C06 / C07 | REAL PASS / TEST SCOPE | 修复版新任务回填一次、发送一次；当前测试私聊和固定短文本，不代表任意长文支持 |
| C08 verify | REAL PASS / TEST SCOPE | 新任务自动记录 VERIFIED_OUTGOING；精确正文、历史上下文连续性双重核对，不代表送达/已读 |
| 同一 RUN 重放 | REAL PASS | 新任务完成后重放零原生调用；旧 UNKNOWN 不重发、不静默改记录 |
| G2 绑定衔接 | IMPLEMENTED | 重放 baseline→verified 已验收连续性，保留 durable key；不猜测新目标 |

实现与范围见[G3c-1 说明](../development/G3C1_SINGLE_REAL_SEND.md)。[收口回执](../acceptance/receipts/2026-10-01-g3c1-closeout.md)记录 853208 自动完成及重放零发送、Keychain 超时和历史 OCR 空格修复，13/13 回归通过（152/61/116）。G3c-1 在当前测试范围收口；真实模型、持续 AUTO_REPLY、多会话不继承本次 PASS。

### G3c-2 单次真实消息与 HTTP 回复联调

| 对象 | 实现 | 边界 |
| --- | --- | --- |
| 原生私有 read | IMPLEMENTED | 原文、方向与签名同一解析投影；仅私有 IPC，公开输出脱敏 |
| arm / once | REAL PASS（单条私聊） | 不回填历史，仅处理一个新增 incoming；新版普通 SQLite 与先落盘的 generation claim，旧加密目录不转换 |
| HTTP 回复与发送 | REAL PASS（单条短回复） | 真实 Chat Completions 请求 1 次；原生填入、单次发送并观察到匹配己方消息；不代表送达/已读 |
| 填入不确定恢复 | REAL PASS（受限） | 仅 `UNKNOWN/fill_uncertain`、无 send receipt、原草稿精确对应、上下文稳定时发送既有草稿；不重新生成或 fill |
| 重放与拒绝分支 | REAL PASS（同 RUN） | 完成后重放模型、read、inspect、fill、send 均为 0；不支持的正文仍不写入、不截断 |

详细步骤见[G3c-2 联调说明](../development/G3C2_REAL_REPLY.md)。[真实 AI 回复收口回执](../acceptance/receipts/2026-10-03-g3c2-real-ai-reply.md)记录一次真实模型请求、VERIFIED_OUTGOING、同 RUN 零重放和 184/61/118 回归。该 PASS 仅覆盖已绑定测试私聊的一条短单行回复，不自动提升为持续值守、多会话或其它适配器支持。

### G3c-3 多行与较长纯文本回复

| 对象 | 当前状态 | 边界 |
| --- | --- | --- |
| 正文范围 | IMPLEMENTED / OFFLINE PASS | 最多 512 UTF-16、12 行纯文本；LF 可用，CR/Tab/其它控制字符拒绝，不截断 |
| 原生写入 | IMPLEMENTED / OFFLINE PASS | 短单行沿用 Unicode；扩展正文通过有界剪贴板快照、粘贴、完整复制回读并恢复原剪贴板 |
| 上下文门禁 | IMPLEMENTED / OFFLINE PASS | 输入区变高只允许顶部历史被遮住；底部新增/删改/乱序均阻断 |
| C08 回执 V4 | IMPLEMENTED / OFFLINE PASS | exact + continuity + content digest；content 仅处理视觉换行，非空白字符必须一致；V3 保留精确兼容 |
| 真机多行 AI 回复 | NOT_RUN | 必须使用新 RUN、新 incoming 验证一次真实模型、一次 fill/send、VERIFIED_OUTGOING 与零重放 |

实现和验收步骤见[G3c-3 说明](../development/G3C3_MULTILINE_REPLY.md)。在真机回执产生前，不把离线剪贴板、合成 worker 或此前短回复 PASS 外推为真实多行支持。

### 多接口与本地设置（2026-10-03）

| 对象 | 当前实现与范围 |
| --- | --- |
| 多个 AI 接入点 | 本地 JSON 明文 api_key，保存/编辑/复制/删除、默认接口，最多 32 项；不自动故障切换 |
| 设置页 | Rust 内嵌 HTML/CSS/JS，本机浏览器入口；不是完整原生桌面应用或 APK |
| 连接测试 | 显式确认一次固定 API 测试；不读取聊天，不自动保存，日志和导出去 Key |
| 新版 G3c-2 存储 | 正常本地 SQLite，无 Keychain 依赖；原任务固定接口，不因默认或无关接口修改而换模型 |
| 旧数据 | 旧加密入口显式兼容；不覆盖旧 key、不转换历史 UNKNOWN、不把模拟测试当真机验收 |

使用方式与边界见[本地设置说明](../development/LOCAL_SETTINGS.md)。

### G3b IME Evidence Spike（归档；不再是执行门禁）

| 信号 | 分类 | 真机结果 | 能否证明 SAFE |
| --- | --- | --- | --- |
| target app AXFocusedUIElement | CONTEXT / UNAVAILABLE | 微信前台时无可用 focused UI element | 否 |
| system-wide AXFocusedUIElement | CONTEXT / UNAVAILABLE | 仅接受 PID 属于微信的 focused element；本机无值 | 否 |
| AXSelectedTextRange | CONTEXT_ONLY | 微信目标元素不可用 | 否；选区也不是 marked-range |
| TIS current input source | CONTEXT_ONLY | 可识别当前输入源及对应输入法进程 | 否；只说明输入源 |
| recent key/mouse/scroll | POSITIVE_BLOCKER | 可检测最近输入 | 只能阻断，静默不能证明 SAFE |
| input-method on-screen window | POSITIVE_BLOCKER | 历史非组字基线为 0；旧版曾接入 send-gate，现已移除 | 只能阻断；absence 不证明 SAFE |
| NSTextInputClient hasMarkedText/markedRange | AUTHORITATIVE（仅 receiver 自身） | 无法从另一个进程取得微信 receiver | 当前不可用 |

保留独立 `NativeIMEEvidenceProbe` 与 `foxbot-macos-ime-evidence` 及原实验结果，但 send-gate 已不再调用。上表为历史实验分类，不是当前生产要求；不伪造 authoritative SAFE，也不再要求取得它。

[IME Spike 说明](../development/G3B_IME_EVIDENCE_SPIKE.md)与[回执](../acceptance/receipts/2026-10-01-g3b-ime-evidence-spike.md)记录完整实验。
