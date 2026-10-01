# G3b 无人值守独占执行调整回执

- 日期：2026-10-01
- 分支：`refactor/g3b-unattended-execution`
- 被测代码提交：`aafbb76fd44537be2f018130b60f2119219f35c1`
- 证据类型：代码复核、编译单测、合成/本机回环集成；不是微信真机发送验收。
- 结论：**13/13 集成检查 PASS；无人值守 Gate 调整已完成。G3c 真实发送仍待实现/验收。**
- 关联：[调整说明](../../development/G3B_UNATTENDED_EXECUTION.md) · [G3b Gate](../../development/G3B_SAFE_SEND_GATE.md)

## 1. 本次调整

按用户确认的无人值守、FoxBot 独占聊天界面前提，从 Rust LiveTarget/双门禁与 Swift native send-gate 移除 IME 可信证明、正在组字、人工活动和候选窗阻断。没有把 unknown 伪造为 SAFE，也没有新增“证明无人操作”的条件。

保留账号/会话/窗口与布局、权限和前台、写前残留草稿、写后内容匹配、revision、DeviceOwner、显式暂停、UNKNOWN 对账与防重复发送。IME 工具仅作为归档诊断保留，正常门禁不再调用。

## 2. 可复现命令与结果

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
```

| 检查 | 结果 |
| --- | --- |
| Rustfmt | PASS |
| Clippy，all targets，warnings as errors | PASS |
| Rust workspace / all targets | 137 PASS |
| 禁用加密 feature 的拒绝降级专项 | 1 PASS |
| Host / HTTP 工具构建 | PASS |
| G2d synthetic bridge smoke | PASS |
| G3b gate-only smoke | PASS |
| HTTP loopback smoke | PASS |
| Host loopback smoke | PASS |
| Python unittest | 61 PASS |
| 文档检查 | PASS |
| Git whitespace | PASS |
| Swift tests / warnings as errors | 97 PASS |

最终检查过程中 HEAD 和源码均未变化。原始结果位于本机 ignored 目录：

```text
target/g1-integration/befd0f7b2867491abc2fc58114156cee/report.json
source_before = source_after = 62d2360f7c0fad5a5cf1f27c8381bef055e499d7d726f33d2c1ddce27b09d344
source_unchanged = true
passed = true
```

新加的开发交接测试首次按“暂停保留旧待发任务”编写而失败；核对现有 `set_host_paused` 后修正测试预期，未改变暂停机制。最终验证暂停使旧任务失效，恢复不补发旧任务，新任务可执行一次且不能重复发送。

## 3. 未执行与边界

真实聊天读取/回填/发送均为 0；external model request 为 0；Keychain 操作为 0。HTTP 和 Host 集成仅使用本机回环合成服务。

native v2 Gate 在真实微信中的运行本轮 NOT_RUN；C07/C08 发送和结果验证尚未实现。旧版 `COMPOSING_UNVERIFIED` 回执保留历史结果，不被改写为新版 PASS。

下一步为 G3c-1 当前会话：持久化待发任务 → 真实回填 → 内容核对 → 单次发送 → 新己方消息匹配。先固定测试回复，再接真实回复服务，不再进入 IME 或人工并发编辑专项。
