# 多接口与本地设置

日期：2026-10-03。此文替代普通使用时的 Keychain / ledger_key 初始化步骤；历史加密验收记录保持原样。

## 1. 普通用户如何配置

在已经构建好的项目目录中，macOS 双击根目录 `打开FoxBot设置.command`。程序打开浏览器中的本机设置页，不需要编辑 JSON、创建账本密钥、输入钥匙串命令或手动设置文件权限。

在页面点“添加”，填写名称、完整接口地址、API Key、模型名称，点“保存接口”。第一个接口自动设为默认。可继续添加其他接口，分别保存或复制、编辑、删除，并显式选择默认接口。

保存不需要联网。单独点击“测试连接”才发出一次固定的简短 API 请求；界面先提示可能产生少量费用。测试不读取聊天、不发送微信消息、不使用聊天历史，也不会自动保存尚未保存的表单。认证、地址/模型、限流、网络与协议错误有简短中文提示；测试失败仍然可以保存配置。

这里是最小本机设置页面，不是完整桌面应用安装包，也没有启动聊天或持续自动回复按钮。原生入口/发布打包、聊天选择和持续值守另行推进，不能将这个页面描述为完整产品已经交付。

开发者首次构建与备用启动命令：

```bash
cargo build --locked -p foxbot-host
target/debug/foxbot-host settings
```

可显式指定另一份配置（测试与便携目录用途），但不会自动搜索其他应用的 Key：

```bash
target/debug/foxbot-host settings artifacts/local/config.json
```

页面“关闭设置服务”结束本次服务；直接关闭浏览器标签页不会结束终端中的服务，也不会启动自动回复。终端 Ctrl-C 同样可以关闭。

## 2. 文件保存在哪

默认配置由平台用户配置目录决定。macOS 是 `~/Library/Application Support/FoxBot/config.json`。页面有“打开配置目录”按钮；开发者可用 `foxbot-host config-path` 查询实际路径。

**API Key 以明文保存在该文件中。能读取这个文件的人也能读取 Key。** 普通用户不需要记住这个路径，软件自动创建目录和首次空配置。新建目录和文件在 Unix 下采用用户私有权限；日志不打印 Key，界面默认不返回已保存的 Key，只显示“已保存”。可显示/隐藏新输入的 Key；编辑时留空保持原值，明确选择“无需 API Key”才清除它。

“导出配置（不含 Key）”会移除所有 API Key，复制接口则在本地保留 Key。更改请求目的地址时需要重新输入 Key 或明确清空，避免把原 Key 不经意发送到另一家服务。请勿把原始配置、数据库或整个用户目录当成诊断包公开分享。

任务状态仍然必须持久化。新版 G3c-2 在配置同级目录的 `runs/<测试会话>/<任务编号>/runtime/ledger.sqlite3` 保存普通 SQLite 账本；`run.json` 保存接口标识、摘要与阶段。它是按任务隔离的实现，并非一个已完成的全局聊天历史管理器。配置、收件消息、AI 回复与发送状态留在本机；Key 不复制到任务账本或普通报告。

## 3. 多接口怎样影响当前任务

普通用户只需要选择默认接口。任务准备时固定该接口 ID 和有效配置，之后切换默认接口、改另一个接口、改显示名称，不会把当前回复中途切到别家。

已经运行的请求使用启动时持有的配置快照。重启后继续某任务时，仍查找原接口 ID；原接口被删除，或它的地址、Key、模型、回复提示词已经变化时，明确停止，不悄悄使用当前默认接口，也不重新生成/重发旧任务。改变默认接口只影响之后准备的新任务。

第一版不实现自动故障切换，不自动遍历多个服务商，不把所有接口都试一遍。发送任务的持久化、单次执行、UNKNOWN 只读核对和目标会话检查继续保留；没有重新增加 IME 或人工活动检测。

## 4. 配置格式

[本地配置示例](../../examples/config.local.json)是 v2 格式，接口中直接保存 `api_key`。只有地址、Key、模型等实际连接资料需要用户填写；超时、请求数和轮询有程序默认值，不变成必填表格。

Chat Completions 接口填写完整的请求路径。远程使用 HTTPS；当前 HTTP 适配器允许数值 loopback 地址上的 HTTP，例如 `127.0.0.1`，不会自动猜测路径或切换证书校验策略。

BusinessV1 继续支持：切换类型后隐藏模型字段，高级区域填写上下文管理和反馈地址等现有协议选项。自定义服务不额外插入通用提示词。普通模型的回复提示词在页面下方单独保存。

## 5. G3c-2 使用同一份配置

配置检查不读取 Keychain、不请求模型：

```bash
CONFIG="$(target/debug/foxbot-host config-path)"
target/debug/foxbot-host g3c-reply-check "$CONFIG" g2d-test --check-config
```

开发联调仍用既有已绑定测试私聊。先固定当前历史基线，再由对端发一条新消息，最后执行一次回复：

```bash
target/debug/foxbot-host g3c-reply-arm "$CONFIG" \
  target/macos-probe/debug/foxbot-macos-send g2d-test local-ai001 --allow-native-read
# 等待 ARMED_WAITING_FOR_NEW_MESSAGE；测试对端此后发一条新消息。
target/debug/foxbot-host g3c-reply-once "$CONFIG" \
  target/macos-probe/debug/foxbot-macos-send g2d-test local-ai001 \
  --allow-network --allow-single-test-send
```

这个版本仍是短单行、80 UTF-16 单元的受限单次联调；不是持续服务。超长/多行不截断后发送。实际 AI 与微信完整端到端验收仍需真实接口和新的测试消息，本地 HTTP fixture 不能代替这项。

## 6. 历史兼容与回归

普通编译默认不启用 `encrypted-ledger`。旧 v1 配置/账本仅保留显式兼容路径；确需检查旧加密账本时构建 `--features encrypted-ledger`，仍需其原密钥。新版不会覆盖旧 key、读取其他项目密钥或自动导出旧加密数据。旧 UNKNOWN 任务不能通过改配置/换存储路径变成新任务；已知旧测试 RUN 的碰撞会被拒绝。

自动测试用临时用户目录、合成消息与本机模型服务器，不能占用用户正在使用的执行锁。`settings_smoke.py` 验证实际设置 HTTP 服务的增改复制删除、默认选择、明文保存、脱敏读取/导出、连接成功/401 失败、重启持久化和关闭。

```bash
python3 scripts/g1_integration_check.py --with-macos-probe
python3 scripts/settings_smoke.py
```

页面只监听数值 loopback 随机端口，API 使用每次启动的本机访问令牌与来源检查，拒绝外部网页直接读取/修改配置。这个令牌由软件自动处理，不是用户需要配置的另一把 Key。
