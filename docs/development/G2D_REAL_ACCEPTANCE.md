# G2d：真实 Observation 验收工作流

- 日期：2026-09-29
- 状态：验收工具链已实现并通过合成/真实 readiness；真实 Observation PASS 仍需要专用测试会话的人工 ground-truth 和另一测试账号发送一条已知消息。
- 前置：[G2d Observation Bridge](G2D_OBSERVATION_BRIDGE.md) · [G2c 会话身份](G2C_IDENTITY_HOST.md)
- 关联：[能力矩阵](../adapters/CAPABILITY_MATRIX.md) · [readiness 回执](../acceptance/receipts/2026-09-29-g2d-real-readiness.md)

## 1. 私有数据目录

### 推荐：先用状态引导器

~~~bash
python3 scripts/g2d_real_status.py init g2d-test
python3 scripts/g2d_real_status.py status g2d-test
~~~

`status` 不读取或输出正文，只检查私有工作流文件的存在、required tags、expected 填写数量、acceptance、baseline 与 verified 状态。它会返回当前阶段、是否需要人工操作以及下一条安全命令。建议每完成一步都重新运行一次，而不是手工记忆整套流程。

阶段会依次推进：`CAPTURE_CASES` → `LABEL_EXPECTED` → `RUN_ACCEPTANCE/FIX_GROUND_TRUTH` → `READY_BASELINE` → `WAIT_EXTERNAL_MESSAGE` → `COMPLETE`。

所有真实正文和人工标注只允许留在：

~~~text
target/g2d-real/<session>/
~~~

该目录已被 Git 忽略。工具要求目录权限 0700、JSON 文件权限 0600，并拒绝符号链接和路径逃逸。公开 stdout 只输出计数/状态，不输出正文、sender、会话指纹或截图。

## 2. 阶段 A：采样 ground-truth case

先将微信停留在专用测试会话，按场景采样：

~~~bash
cargo run --quiet --locked -p foxbot-host -- \
  g2d-private-capture \
  target/macos-probe/debug/foxbot-macos-ocr \
  g2d-test \
  private-basic \
  private \
  --allow-private-test-data
~~~

命令会连续读取两次，只有 application-session 与 conversation fingerprint 稳定时才写：

~~~text
target/g2d-real/g2d-test/
├── snapshot-private-basic.json
└── groundtruth.json
~~~

`groundtruth.json` 中 `observed` 来自 FoxBot 实际解析，`expected` 永远默认空数组。工具不会把 observed 自动复制成 expected，因此 OCR 不能自己证明自己正确。

推荐至少准备 6 个 case，并由测试人员在本机填写 expected：

| case | tag | 重点 |
| --- | --- | --- |
| private-basic | private | 私聊方向与顺序 |
| group-sender | group | 群 sender 与方向 |
| duplicate-text | duplicate_text | 连续相同短句 |
| numeric-money | numeric | 数字、金额、订单号 |
| multiline | multiline | 同一气泡多行 |
| reference | reference | 引用/回复上下文 |

总 expected 消息不少于 24 条。

如只是检查当前窗口可否稳定采样，可以使用额外标签 `readiness`；该标签不计入最终 6 类覆盖。

## 3. 阶段 B：人工标注与 acceptance

推荐不要直接编辑 JSON，而是运行本机交互式审核器：

~~~bash
python3 scripts/g2d_review_ground_truth.py g2d-test
~~~

审核器会在用户自己的终端中逐条显示 `observed` 的文字、ME/THEM 与 sender_labeled，并提供：

- `Y/Enter`：确认该条识别正确；
- `E`：修改文字、方向或 sender_labeled；
- `D`：把误识别条目从 expected 中删除；
- 额外补加：录入 OCR 完全漏掉的消息。

正文只显示在真实交互 TTY。若 stdin/stdout 不是 TTY（例如 WebCodex、管道、CI 或日志采集），审核器会直接返回 `REFUSED_NON_INTERACTIVE`，避免真实聊天正文进入自动化日志。

审核完成后工具原子写回 `target/g2d-real/g2d-test/groundtruth.json` 的 expected。然后运行：

~~~bash
python3 scripts/g2d_ground_truth.py g2d-test
~~~

它复用 G2c 的严格门槛：

- ≥ 6 case；
- ≥ 24 条 expected；
- 覆盖 private/group/duplicate_text/numeric/multiline/reference；
- direction error = 0；
- sender error = 0；
- message-count error = 0；
- text error ≤ 2%。

输出只含误差统计，并写：

~~~text
target/g2d-real/g2d-test/acceptance.json
~~~

如果 `accepted=false`，`g2d-real-baseline` 会在启动 OCR worker 之前拒绝，不继续读取聊天。

## 4. 阶段 C：建立显式 Binding 与 baseline

只有 acceptance 已通过时：

~~~bash
cargo run --quiet --locked -p foxbot-host -- \
  g2d-real-baseline \
  target/macos-probe/debug/foxbot-macos-ocr \
  g2d-test \
  test-account \
  test-conversation \
  private \
  --allow-private-test-data
~~~

`test-account` / `test-conversation` 是 FoxBot 测试用稳定 opaque ID，不要求写真实联系人名称。

成功后私有目录增加：

~~~text
bridge-config.json
baseline-snapshot.json
~~~

Binding 同时固定：stable ConversationKey、identity_epoch、当前 application-session fingerprint、当前 conversation fingerprint、accepted ground-truth revision。

baseline 中 UNKNOWN direction 或不完整消息会直接拒绝。

## 5. 阶段 D：发送一条已知外部测试消息

baseline 完成后，不操作本机输入框。

从另一个测试账号向当前测试会话发送恰好一条预先知道内容的 incoming 消息。不要同时滚动、切换联系人或发送其他消息。

随后运行：

~~~bash
cargo run --quiet --locked -p foxbot-host -- \
  g2d-real-verify \
  target/macos-probe/debug/foxbot-macos-ocr \
  g2d-test \
  --allow-private-test-data
~~~

PASS 条件固定为：

~~~text
baseline.bridge_state  = BASELINE
current.bridge_state   = NEW
current.observations   = 1
current.queued         = 1
repeat.bridge_state    = NO_CHANGE
repeat.observations    = 0
~~~

其他结果全部 fail-closed。

## 6. Runtime 数据安全

真实 verify 不使用明文 simulation ledger。

每次验证都会：生成随机 32-byte key → 打开一次性 SQLCipher Runtime → ingest baseline/current → 重读 current → 关闭 Runtime → zeroize key → 删除临时 Runtime 目录。

只有整个验证 PASS 后才保存 `verified-snapshot.json` 到私有 session 目录，用于本机审计。

整个工作流不调用 ReplyProvider、不发 HTTP、不创建回复任务、不调用原生 fill/send。

## 7. 当前实机 readiness

固定代码提交的真实微信 readiness 已成功采样：

- 连续两读稳定；
- observed message count = 9；
- 公开输出 `raw_text_included=false`；
- 私有文件权限检查通过；
- 因 expected 为空，acceptance=false；
- 使用不存在的 worker 路径调用 baseline 仍先返回 `Untrusted`，证明未 accepted GT 会在启动 worker 之前被拒绝。

该 readiness case 使用 tag `readiness`，不是 ground-truth 通过证据。

实际 `duplicate_text` 场景暴露了微信宽窗口标题可能位于 top-left 归一化 x≈0.81 的布局。标题识别仍限定在顶部标题带 y<0.10，但横向范围从 x<0.66 扩展到 x<0.94；真实场景重新采样后通过。该调整没有扩大到正文 y 区域，也没有使用联系人文本本身作为日志或公开输出。

后续真实群聊又暴露两个布局/窗口边界：群标题可从 x≈0.28 开始，因此 OCR readRegion 左边界扩到 0.27，但消息 chatRegion 仍保持 x>=0.32；同时 ScreenCaptureKit 可能返回一个 on-screen 主窗口和多个同几何 off-screen AppEx 镜像。focused 模式只在同框候选中存在唯一 on-screen 窗口时进行消歧；若存在多个 on-screen 候选仍保持 AMBIGUOUS_WINDOW。

## 8. 下一步

现在代码侧已经准备好。真实 G2d 剩余工作是测试人员本机完成：

1. 专用测试会话采 6 类 case；
2. 人工填写 expected；
3. acceptance 通过；
4. baseline；
5. 另一账号发送一条已知 incoming；
6. real verify PASS。

这一步完成后，才可以把 G2 真实读取链标为 Freeze 候选；G3 写入/发送仍是独立门禁。
