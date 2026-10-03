# 多接口本地配置与设置页面验收

- 日期：2026-10-03。
- 开发基线：G3c-2 分支 `0907dea1c9acb702f0e5e050d910252c3eed1040`；合入前 main 为 `99e36cb3b502f3d19409ea090a22ba9ae272bee1`。
- 被测代码：`2926c44f81ed2b783a89228502a298dbd5f1e73b`。
- 结论：**本地多接口配置、普通 SQLite 与最小设置页面实现通过回归；实际设置 HTTP 服务和浏览器呈现已检查。真实 AI＋微信自动回复仍未执行，不继承本次 PASS。**
- 使用说明：[多接口与本地设置](../../development/LOCAL_SETTINGS.md)。

## 1. 用户约定与实际实现

默认版本在用户目录 `config.json` 明文保存多份连接的 API Key，正常新任务使用普通 SQLite，不再要求 init-key、set-token 或 Keychain 授权。根目录增加可双击的 `打开FoxBot设置.command`，已构建的宿主自动启动仅本机可见的设置页。

页面支持接口增删改、复制、显式默认选择、单独保存回复提示词、打开配置目录、导出不含 Key 的配置。保存不访问服务；测试连接单独确认一次 API 调用，使用固定测试文字，不带聊天历史，不写入聊天软件。已保存 Key 不返回到页面正文，留空表示保持原值，明确空值表示清除；复制接口在本机保留 Key。更改目标地址时不自动把旧 Key 转交新地址。

当前任务按接口 ID 和有效配置固定：换默认、改无关接口、改显示名不使已有任务换服务；原接口被删除或语义/凭据改变后，重新执行旧任务明确拒绝，而不是切到其他接口。没有实现自动失败转移。原 Runtime 的发送记录、执行锁、幂等和 UNKNOWN 不重发逻辑保留。

普通构建不默认启用 SQLCipher；旧加密代码和回归仍以显式 feature 保留，不覆盖旧账本。新 open_local 在打开旧非 SQLite 明文文件前拒绝，测试确认文件内容不变、未创建其 owner.lock。旧任务编号在已知旧 G3c-2 目录中的冲突不会自动创建新版副本。

## 2. 固定提交完整回归

命令：`python3 scripts/g1_integration_check.py --with-macos-probe`。

最终报告：`target/g1-integration/e8e3402c078647d7ba30fafa0d971bfd/report.json`。

- head/head_after：上述 `2926c44` 完整提交。
- source_before/source_after：`22e5beb1f17a23d98bc1bd83dd291c432016d3e2120559a2b88420e20f8011d1`。
- source_unchanged=true，passed=true，**16/16 检查通过**。

| 检查 | 结果 |
| --- | --- |
| Rust 工作区全部 targets，含显式旧加密 feature | 178 PASS |
| 不启用默认/加密 feature 的 G3c-2 子集 | 16 PASS |
| 不启用默认/加密 feature 的设置子集 | 10 PASS |
| 禁用加密时旧加密入口明确拒绝 | 1 PASS |
| Python | 61 PASS |
| Swift warnings-as-errors | 117 PASS |
| Rustfmt / Clippy / 工具构建 / 既有 bridge、gate、HTTP、host smoke | PASS |
| 新 settings-smoke / 文档 / whitespace | PASS |

16/10 是同组用例在无加密构建下的额外执行，不应与 178 相加宣称独立测试数。默认构建出的工具用于实际设置 smoke，不是用加密构建掩盖普通 SQLite 路径。

新回归覆盖内联 Key 落盘、UI/导出脱敏、保留/清除语义、默认选择、复制/删除、失效保存不覆盖、改变地址不自动外传旧 Key、损坏配置不被重置、跨来源/API 令牌校验、固定文字连接测试与错误提示、不联网保存、任务连接固定、普通账本重启和旧数据保护。

## 3. 实际 HTTP 与浏览器检查

`settings_smoke.py` 启动实际编译宿主、临时配置目录和本机模拟模型服务器；执行两份接口保存、修改、复制、删除、默认切换、一次成功测试和一次 401 测试，检查保存不触发请求、Key 只在本地配置存在、公开响应和导出不含 Key，关闭并重启后仍保留两份配置。

每次脚本结果：settings_http_smoke=PASS，loopback_generation_requests=2，external_model_requests=0，native_chat_operations=0，keychain_operations=0。模拟响应不是实际 AI 生成。

另运行 `python3 scripts/settings_smoke.py --preview-seconds 90`，在本机 Chrome 打开同一设置页面，实际看到两份测试接口、默认标记、编辑表单、已保存 Key 的留空保留提示、提示词区和关闭服务按钮。窗口截图检查未见内容溢出或缺失，页面脚本确已加载服务保存的数据；这不是每个按钮都经过人工逐项点击的声明，操作链由 HTTP smoke 覆盖。

窗口快照摘要：`2214824962f19c85e9948bff292d04dcfe91e3986d9a740880e92c5d84b3c2a4`。截图未纳入仓库或分发包。预览限定 90 秒，随后设置服务与模拟服务均关闭，临时数据清理。

JavaScript 使用 `node --check` 通过；Python smoke 源码编译与文档链接检查通过。

## 4. 保留失败与修复过程

首次集成报告 `66c9ddc6a21643cd8ac4d3e874bf3716` 的进程锁测试失败：用户终端中的旧 `set-token foxbot-g3c2-api` 命令正在持有真实执行锁。未终止该进程，也未放松产品执行锁；合成进程测试与 host smoke 改用临时 HOME/用户数据目录，在自己的作用域内继续验证跨账本互斥。

第二次报告 `8a3e424542ab4cf398acbaa55561a4c5` 出现设置复制测试的 Busy。配置文件锁增加显式解锁 guard，避免仅依赖最后一个文件描述符关闭；后续完整回归通过，保存仍保持互斥和版本检查，不自动覆盖并发修改。

第三次报告 `b82803e4ec204d38b7486df2526d6151` 的前三个实际 Vision 合成文本识别用例返回 recognitionFailed（其他步骤通过）。本轮未修改 OCR 算法、预算或验收条件。随后同源 OCRTests 14 项复查通过，固定提交完整 117 项再次通过；先前失败记录保留，不将其描述为已验证的具体系统根因或全程一次通过。

## 5. 产物与未覆盖范围

2026-10-03T11:10:15+08:00 复核默认构建 `target/debug/foxbot-host` SHA-256：`15c1294034fc598ccfb4ad1f7bcbc67bcee7540ea1b2f17059b7a01ed5449e74`。

本轮未读取或迁移用户真实 API Key，未操作 Keychain，未调用真实模型，未读取/回填/发送微信消息；没有重试历史任务。临时测试配置不是用户默认配置文件。用户自己的旧 set-token 终端进程未被本轮终止，后续可由用户 Ctrl-C 退出；新版配置无需它。

尚未完成完整桌面安装包、聊天启动界面、持续自动回复、多会话、长文本/多行、移动端及其他系统真机验收。现有 G3c-2 仍是受限当前测试私聊的一条短回复。下一步由设置页填入真实接口并保存，再在原测试私聊按新基线完成实际 AI 回复联调。
