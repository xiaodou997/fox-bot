# FoxBot 适配能力矩阵

- 编号：FB-MATRIX-001
- 日期：2026-09-28
- 适用设计：[FB-BASELINE-001](../design/BASELINE.md)
- 状态：所有 FoxBot 适配器均未实现、未真机验收。上游源码只能作为路径参考。
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

目前只能声明：已确定范围、已审阅上游固定路径、已设计能力分解。尚不能声明任何 FoxBot 应用适配器可读、可发、可值守或已经通过真机测试。
