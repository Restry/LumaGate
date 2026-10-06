# 为 LumaGate 贡献

请向 [Restry/LumaGate](https://github.com/Restry/LumaGate) 提交问题与 Pull Request。先阅读 README、PRODUCT.md、DESIGN.md，保持 Provider → 模型 → 客户端的显式工作流。

## 验证

```sh
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test:manual
```

修改 Rust 手动网关逻辑时，可在隔离环境针对性运行 `cargo test --release --manifest-path src-tauri/Cargo.toml --lib manual:: -- --test-threads=1`。正式发布由 Tauri 原生构建证明编译通过，不重复编译完整上游测试套件。

当前应用入口为 `src/manual/ManualApp.tsx` 与 `src-tauri/src/manual/`。共享协议模块沿用已有实现，不另起转换引擎。历史上游界面并非当前运行入口；请不要用其截图或文案描述本产品。

## 分支与发布

- 仓库仅保留 `dev` 与 `release`；日常开发和 PR 目标为默认分支 `dev`。旧历史位于只读归档 `Restry/LumaGate-legacy`，不推送或重写该仓库。
- 合并前在本地完成上述检查和相关测试；推送 `dev` 和提交 PR 不运行自动 CI。维护者正常推进 `release`；GitHub Actions 仅保留版本保留、正式构建、产物校验与发布，不能 force push。
- 修改版本时同步 `package.json`、`src-tauri/tauri.conf.json`、Cargo package 和 Cargo.lock 的 `lumagate` 条目。内部库继续叫 `cc_switch_lib`。
- 运行 `python3.13 scripts/releasing/release.py check` 与 `python3.13 -m unittest discover -s scripts/releasing -p 'test_*.py' -v`。这些离线测试不代表真实发布成功。
- 发布条件、资产校验、重跑及签名限制见 [docs/RELEASING.md](docs/RELEASING.md)。

## 必须保留的边界

- 无自动推理测试、自动同步客户端、偷偷重登录或轮换凭据。
- 配置写入须保留 revision 冲突检测，失败保留最后成功数据。
- 不触碰独立上游数据目录。迁移需私有备份、停写、SQLite WAL 与完整性核验、权限及密钥文件校验，禁止覆盖目的数据。
- 测试只使用隔离数据，真实账号操作必须获得明确授权。不要提交凭据或原始用户日志。
- 保留 MIT 和原始版权。不得将历史资料伪装成当前产品承诺。
- 继承的 Gemini 订阅查询模块不随包携带上游 OAuth 客户端凭据。其可选刷新仅读取 `LUMAGATE_GEMINI_OAUTH_CLIENT_ID` / `LUMAGATE_GEMINI_OAUTH_CLIENT_SECRET`，缺失或为空时不发送刷新请求；当前手动网关不调用该模块。不要在源码或 CI 配置中补回凭据。

视觉改动沿用 Geist、中性表面、黑白主操作、蓝色图表；检查窄 / 短窗口、深浅主题、键盘焦点、错误恢复与减少动态效果。说明实际验证范围，不将模拟 IPC 当作真实后端证据。
