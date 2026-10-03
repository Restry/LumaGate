# 枢光 · LumaGate

本机模型网关，供 **Pi、Claude Code、Codex** 统一接入普通 API Provider 与个人 GitHub Copilot。

[源码](https://github.com/Restry/LumaGate) · [问题反馈](https://github.com/Restry/LumaGate/issues) · [发行包](https://github.com/Restry/LumaGate/releases)

## 使用

1. 在 **Provider** 添加支持 `/v1/models` 的服务，或明确连接 GitHub Copilot。
2. 点击 **刷新模型** 获取账号实际目录。获取目录不发推理请求、不自动同步客户端。刷新保留已有模型启停选择；失败保留最后一次成功目录。Copilot 使用已保存登录，只有上游拒绝授权时才需要重新登录。
3. 在 Provider 标题直接启用 / 停用来源；停用退出后续路由，但不删除模型、密钥、登录或历史。模型选择弹窗仍需显式保存。
4. 在 **模型** 查看元数据、配置路由或全局屏蔽模型。同模型同 Endpoint 聚合；默认轮询与故障切换，跨模型 Fallback 须明确设置。名称歧义不自动猜来源。
5. 在 **客户端** 预览同步并确认写入。每次写入先备份，检查文件和配置版本。普通启动只运行客户端，不修改配置或默认模型。

**模型测试可能计费。** 每次点击只向指定来源发送一次文本请求；不自动巡检、不切换来源替它通过。本次目录刷新无需测试模型。

## 本地数据与迁移

- 可执行文件：`lumagate`；应用标识：`cn.restry.lumagate`；macOS 应用：`/Applications/LumaGate.app`。
- 规范数据目录：`~/.lumagate/`。数据库内部文件名 `cc-switch.db` 保留，以避免破坏已有数据库备份契约；它不代表另一款应用的数据目录。
- Provider 密钥：`keys/`；Copilot 登录：`copilot/`；内网密钥：`access-keys/`；完整调用历史：`logs/requests.sqlite3`。
- 数据未加密，Unix 私有目录及密钥分别使用 `0700` / `0600`。备份含凭据，不能放进 Git 或公开分享。
- 旧的自身目录 `~/.cc-switch-manual` 在启动前迁移：拒绝活动日志写入者，检查 SQLite 完整性并处理 WAL，私有备份到 `~/.local/share/lumagate/migration-backups/`，核验文件内容和权限后整目录迁移。回执在同级 `migration-receipt.json`。新旧目录同时存在时拒绝覆盖或合并。
- **不读取、不迁移、不清理独立的 `~/.cc-switch`。** 不以永久旧名符号链接维持运行。
- 稳定模型路由 ID、账号绑定、密钥派生盐、占位连接 token 与现有 `cc_switch_manual` 客户端 profile 保持兼容。安装、启动、刷新和停用不会改写用户 Claude/Codex/Pi 文件；无需重新同步或轮换密钥。

macOS 更换 bundle identifier 前应退出旧应用，并将自身的 WebKit / Application Support 存储安全迁移到新标识。遇到冲突或权限不足必须停止，不能创建空配置覆盖原数据。

## 网络与安全

默认 `127.0.0.1:15722`，启动应用不自动启动网关。可在设置明确选择 IPv4 内网监听，保存后手动重启。监听地址和实际运行地址分别显示。

- 本机目录与推理免授权；非回环连接必须来自私有 / 链路本地 IPv4，并验证独立访问密钥。
- 不信任转发头，不接受网页跨源调用，保留 Host / Origin 检查；禁止把免鉴权回环入口反代到公网。
- 内网 HTTP 未加密，仅用于可信网络。应用不修改防火墙、开放公网或自动同步 LAN 密钥。
- 上游密钥只由后端读取，不返回普通快照、写入客户端配置或日志。编辑密钥留空表示保留。
- Copilot 使用编辑器兼容接口，不是 GitHub 官方网关；授权页可能显示 VS Code。只使用当前个人账号可用目录，不轮换账号、不规避额度、不猜模型。

## 日志、能力与界面

概览和日志只读本地 SQLite 全历史，支持日期、模型、来源、密钥、接口、状态及请求 ID 筛选；统计覆盖全部匹配结果而非当前页。日志停止 / 重启后保留，不自动清空。正文只保留有界脱敏预览，仍可能包含业务内容。

HTTP 200 不等于生成成功：协议完成状态、传输结束和 Token 报告分别记录。未知用量不填零，缓存不重复累加；图表不冒充账单或全部重试费用。

模型能力来自真实目录，缺字段显示未知。既有用户指定限额规则、图片来源筛选、会话绑定、Responses 续轮和故障回退边界保留；旧路由 ID 继续有效。详细约束见 [PRODUCT.md](PRODUCT.md) 与 [DESIGN.md](DESIGN.md)。

界面保留 Geist、中性表面、黑白主操作与蓝色图表，覆盖概览、日志、Provider、模型、客户端、设置、帮助。紧凑布局不靠缩小中文正文，保留焦点、深浅外观与减少动态效果。

## 开发与构建

依赖 Node / pnpm、Rust 1.95、Tauri 2 平台构建工具；支持任意检出路径。

```sh
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test:manual
pnpm tauri build --bundles app
```

macOS 若系统 Xcode 未就绪，可在命令上设置 `DEVELOPER_DIR=/Library/Developer/CommandLineTools`，不修改全局工具链或以 root 构建。迁移源码目录后，Tauri 构建缓存中的绝对路径需要重新生成。

`pnpm preview:manual` 是隔离夹具 UI，不连接真实账号，不能充当生产后端证明。真实验收需要实际应用、已授权账号与私有数据校验。

## 开发分支与正式发布

仓库仅保留 `dev` 与 `release` 两个分支，`dev` 是默认开发分支。维护者将完成的提交正常快进或合并到 `release`；发布流程自行运行轻量检查，不等待 dev 重复构建。推送后由 GitHub Actions 自动构建并发布，不手工覆盖版本标签或已公开资产。

首个版本基础为 `3.24.0`，四处版本清单必须一致。后续同一 minor 的新提交自动分配递增 patch；同一源码 SHA 重跑复用版本。正式资产包含 macOS arm64/x64 DMG、Windows x64 NSIS、Linux x64 AppImage/deb、`BUILD-INFO.json` 与 `SHA256SUMS`。下载只认仓库 [Releases](https://github.com/Restry/LumaGate/releases)，不把 Actions 临时产物当正式发行。

macOS 仅 ad-hoc 签名、未公证；Windows 未做 Authenticode 签名。完整流程与失败恢复见 [发布指南](docs/RELEASING.md)。

## 授权与历史

LumaGate 基于 [CC Switch](https://github.com/farion1231/cc-switch) 的 MIT 代码发展而来，并非全部独立创作。保留 [LICENSE](LICENSE) 中 Jason Young 的原始版权；内部共享 Rust 库名 `cc_switch_lib`、仍参与编译的历史模块及协议兼容标识不作无意义改名。随应用分发的 Geist 字体授权见 `src/manual/assets/GEIST-LICENSE.txt`，第三方依赖遵循各自许可证。

本仓库从当前产品代码建立全新历史。完整旧历史、旧手册和设计试验保留在只读归档 [LumaGate-legacy](https://github.com/Restry/LumaGate-legacy)，不再打包进当前源码。当前产品问题请提交到 Restry/LumaGate。
