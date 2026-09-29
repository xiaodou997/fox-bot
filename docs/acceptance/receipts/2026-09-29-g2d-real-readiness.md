# G2d 真实验收工具链 / Readiness 回执

- 日期：2026-09-29
- 被测代码提交：`1c079300ea5e279279b66198d948bf1e2b43d50f`
- 分支：`feat/g2d-real-acceptance`
- 环境：macOS 27.0 / arm64，微信 4.1.13
- 结论：**真实验收工具链 PASS；当前微信 readiness PASS；真实 ground-truth / Observation 仍 BLOCKED。**

## 1. 固定提交集成门禁

`python3 scripts/g1_integration_check.py --with-macos-probe`：

- Rust：126 项通过；
- cipher-disabled：1 项通过；
- Python：51 项通过；
- Swift：71 项通过；
- Rustfmt / Clippy / build-tools / g2d-bridge-smoke / HTTP smoke / host smoke / docs / whitespace：通过；
- source fingerprint 前后均为 `cc499655a4e7d66c19b9010167c529a4d53c98d5116b267d8ee19df65145f514`；
- external model requests：0；
- native chat operations：0。

## 2. 私有 ground-truth 评估器

使用 6 个合成场景 / 24 条消息实际运行 `scripts/g2d_ground_truth.py`：

- accepted=true；
- 6 个 required tags 全覆盖；
- direction/sender/count/text error 均为 0；
- acceptance 文件权限为 0600；
- stdout 不含 raw expected/observed。

该合成结果只验证工作流和门槛，不代表真实微信准确率。

## 3. 固定提交真实 readiness

固定 SHA 上运行 `g2d-private-capture`，session=`fixed-readiness-20260929`、case=`readiness`、tag=`readiness`。

公开结果：

~~~text
status                 PRIVATE_CASE_CAPTURED
observed_messages      9
stable_two_reads       true
raw_text_included      false
private_file_written   true
~~~

真实正文只存在本机 Git-ignored 目录：

~~~text
target/g2d-real/fixed-readiness-20260929/
~~~

本回执没有读取或复制其中正文。

权限检查：

- session directory：私有；
- groundtruth.json：0600；
- snapshot-readiness.json：0600；
- acceptance.json：0600。

## 4. 自我验收防护

readiness case 的 expected 默认空数组。

运行 ground-truth evaluator 得到：

~~~text
accepted             false
cases                1
labeled_messages     0
covered_tags         readiness
message_count_error  1
raw_text_included    false
~~~

随后使用不存在的 worker 路径调用 `g2d-real-baseline`，结果仍首先返回：

~~~text
native snapshot identity or trust is insufficient
~~~

这证明未 accepted ground-truth 时 baseline 会在 NativeReadHost/worker 启动前 fail-closed。

## 5. 合成完整 real workflow

Rust 单元测试实际执行：

~~~text
accepted ground-truth
→ prepare_baseline
→ encrypted ephemeral Runtime
→ baseline ingest
→ exactly one new incoming
→ queued = 1
→ repeat same snapshot
→ NO_CHANGE
→ close Runtime
→ zeroize key
→ delete runtime directory
~~~

同时验证私有 capture 不会自动填 expected。

## 6. 当前 BLOCKED

真实 G2d Observation PASS 尚未执行，因为：

- 没有人工完成 6 类真实 ground-truth；
- 没有 accepted 真实 acceptance.json；
- 没有为测试会话建立 baseline；
- 没有从另一测试账号发送“一条已知 incoming”后的 real verify。

因此不能把 readiness 或合成 workflow 记成真实 Observation PASS。

## 7. 操作边界

本轮：

- AI / ReplyProvider：0；
- HTTP 模型调用：0；
- 微信输入：0；
- 微信点击发送：0；
- 原生 write/send：0；
- 真实私有文件未提交。
