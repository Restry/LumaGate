# LumaGate 发布流程

## 日常开发与下载

- `dev` 是日常开发分支和仓库默认分支。推送 `dev`、向 `dev`/`release` 提交 PR 会运行 `LumaGate CI`，不会发布。
- CI 只运行 TypeScript 检查、当前 gateway UI 的 `pnpm test:manual`、四处版本与发布工具契约测试。不在 dev 编译安装包，也不运行完整上游 cargo test、clippy 或 rustfmt 门禁；renderer 与 Rust 的真实编译由 release 的 Tauri 构建完成。已有手动 Rust 模块验证可复用，修改相关逻辑时按需本地运行 `--lib manual::`，不在每个平台重复编译测试套件。
- 将已完成变更从 `dev` 合入 `release` 并推送，触发 `LumaGate Release`。不需要手动打标签。
- 发布流程在同一个 workflow 中依次完成门禁、版本保留、五个目标构建任务、完整性校验、草稿上传、正式发布。不会依赖 `GITHUB_TOKEN` 创建标签后触发另一条 workflow。
- 用户最终从 <https://github.com/Restry/LumaGate/releases> 下载。Actions artifacts 仅用于任务交接，保留 14 天，不是最终下载地址。

## 版本与源码关系

基础版本来自 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 Cargo.lock 中的本地应用包，四处必须一致且为稳定 `MAJOR.MINOR.PATCH`。

独立标签命名空间为 `lumagate-vX.Y.Z`，不使用/覆盖上游历史 `v*` 标签：

1. 首次发布使用经过校验的基础版本；LumaGate 首发集成的四处基础版本为 `3.24.0`。
2. 同一 major/minor 系列后续新 SHA：取基础版本与已有保留版本下一 patch 的较大值。按数字比较，不按字符串比较。
3. 显式 minor/major 发布：在 `dev` 将四处基础版本同步到新的 `X.Y.0` 或 `X.0.0`，提交并合入 `release`。只写 conventional commit 的 `feat!:` 不会自动改版本。不要将基础版本回退到已经发布过的更低 major/minor。
4. 门禁成功后，用 annotated tag 保留版本，标签直接指向触发事件的完整源 SHA，annotation 记录版本与 SHA。标签不是 Release。构建失败会留下保留标签，可能出现版本空号；不得删标签回收版本。
5. 同一 SHA 的重跑/手动重试复用原版本和原标签，包括较旧失败运行的重跑。成功发布后不重新构建、不覆盖资产，而是下载原始发布资产核对 SHA、版本、清单和校验和。
6. 仅在 GitHub runner 的构建 checkout 修改上述四处版本；不提交 bump commit，不修改依赖版本，不形成 CI 循环。Rust `CARGO_PKG_VERSION` 与 Tauri runtime/安装器版本因此一致。`BUILD-INFO.json` 将版本映射回精确源码 SHA、目标架构及各平台文件摘要。源码标签中的 manifest 仍是基础版本；复现时须先执行 `stamp`。
7. 为兼容 Windows installer，major/minor 最大 255、patch 最大 65535；到界限必须显式升 minor/major，不能自动溢出。

整个发布 workflow 使用固定 concurrency group、`cancel-in-progress: false`、`queue: max`。GitHub 最多保留 100 个 pending 运行；超过容量的运行可能取消，需要在原运行上重试。按进入等待队列的顺序处理，不承诺按 push 时间排序。旧版本补发不会将 Latest 从更高版本移回去。

## 必需资产：六个安装包 + 两个校验文件

下表以 `VERSION` 表示分配后的版本；新正式 Release 必须恰好有 **8 个上传资产**。GitHub 自动生成的 Source code zip/tar 不计入此数。代码从平台清单推导数量。

| Runner | Rust target | 必需文件 |
| --- | --- | --- |
| `macos-15` | `aarch64-apple-darwin` | `LumaGate-VERSION-macos-arm64.dmg` |
| `macos-15-intel` | `x86_64-apple-darwin` | `LumaGate-VERSION-macos-x64.dmg` |
| `windows-2022` | `x86_64-pc-windows-msvc` | `LumaGate-VERSION-windows-x64.exe`（NSIS 安装器） |
| `windows-2022`（x64 主机交叉编译） | `aarch64-pc-windows-msvc` | `LumaGate-VERSION-windows-arm64.exe`（NSIS 安装器） |
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `LumaGate-VERSION-linux-x64.AppImage`、`LumaGate-VERSION-linux-x64.deb` |
| 汇总 | — | `BUILD-INFO.json`、`SHA256SUMS` |

使用 Tauri 配置及目标 bundle 目录发现产物，不硬编码旧 `cc-switch` 可执行文件名。每种格式必须恰好匹配一个文件，缺失、重复、跨 SHA/版本/平台混装、未知额外文件、空文件、摘要不符均阻止发布。macOS 检查 app 内版本、Mach-O 架构、结构性 codesign 验证及 DMG 校验；Windows 用 runner 预装的 7-Zip 解压 NSIS，检查其中主程序的 PE Machine（x64 `0x8664`、ARM64 `0xAA64`）、ProductVersion/ProductName，并核对其与构建主程序的摘要/大小一致；Linux 检查 deb 版本/架构和 AppImage ELF 架构。检查不会启动应用或接触用户数据。

macOS 明确构建 `app,dmg`，保留 `.app` 供版本、架构与签名检查；仅构建 `dmg` 会被 Tauri 清理中间 `.app`。公开资产仍只收集两个架构的 DMG，不额外上传应用目录。

原始输出只上传本次 workflow 的平台 artifact。汇总不合并同名目录，先检查每个平台自己的 `build.json`，再生成所有安装包及 `BUILD-INFO.json` 的 `SHA256SUMS`（不包含自身）。上传草稿后再次检查 GitHub 返回的完整资产名称、推导数量、大小、上传状态及 SHA256 digest，最后才将 `draft=false`、`prerelease=false`。任何必需平台失败都不能发布残缺版本。

新 `BUILD-INFO.json` 使用 `schema: 2`，要求全部五个目标，并记录 Windows 安装器与解压主程序的版本、PE Machine、摘要和大小回执。无 schema 的原契约仅用于 `3.24.2` 及以前版本，仍按原四目标资产验证；新版本缺失 schema 或 ARM64 不能降级通过。历史公共资产、校验和及标签不重写。

Windows ARM64 使用已有 `windows-2022` 上的 `Microsoft.VisualStudio.Component.VC.Tools.ARM64` 与 `rustup target add aarch64-pc-windows-msvc`。正式编译前用同目标编译/链接一个最小 Rust 程序，尽早暴露工具链问题。Rust 与 Tauri 官方支持 Windows 主机跨架构编译；不依赖本地 VM。GitHub 也提供 `windows-11-arm`，本流程为复用已验证的 Node/Corepack/VS2022 环境而不切换 runner。

## 失败与重试

```bash
# 选定精确源 SHA 的运行，记录 RUN_ID，不要误看另一个 push。
gh run list --repo Restry/LumaGate --workflow release.yml --branch release --commit SOURCE_SHA
gh run view RUN_ID --repo Restry/LumaGate --log-failed
gh run rerun RUN_ID --repo Restry/LumaGate --failed
gh run watch RUN_ID --repo Restry/LumaGate --exit-status
```

- 门禁失败：尚未保留版本，无 Release。
- 矩阵失败：保留标签仍在，成功的平台 artifact 可供同一运行的 failed-jobs 重跑使用；不生成 Release。
- 上传中断：仅留下不可公开下载的草稿，不更新 Latest。下次发布会验证草稿的 tag/SHA 归属，删除该未发布草稿并重新上传完整资产；永远不删除标签、公共 Release 或公共资产。
- 上传失败后的草稿无需手动清理；保留可供诊断。若中断恰好发生于公开成功后，重跑读取公共发布状态，不盲目删除。网络/API 错误不伪装成“没找到”。
- 14 天后 artifacts 已过期：使用 `gh run rerun RUN_ID --repo Restry/LumaGate` 重跑整次运行。不要只重跑 publisher。
- 同一版本不承诺跨工具链重新构建的二进制字节完全相同，所以公共版本只验证原始下载，不覆盖重建文件。
- `workflow_dispatch` 仅允许 `release` ref；在 `dev`、标签或其他分支选择运行会被 guard 拦截。手动运行当前 release：`gh workflow run release.yml --ref release --repo Restry/LumaGate`。恢复旧 SHA 应重跑原 RUN_ID，不要倒退/强推 release 分支。
- GitHub API/权限、runner 可用性、包源网络、构建失败必须修复并重跑；不能删掉失败平台冒充成功。

## 安装与签名边界

不需要新增仓库 secret；只使用 GitHub 自动提供的短期 `GITHUB_TOKEN`。macOS 使用伪身份 `-` 做 ad-hoc 签名，不需要 Apple 证书；这**不是 Apple Developer ID 签名、公证或 Gatekeeper 认可**。首次安装可能被系统阻止，用户应先核对来源与校验和，再按 macOS“隐私与安全性”的单应用允许流程处理。不要全局关闭 Gatekeeper，也不自动清除 quarantine。

Windows 安装器未做 Authenticode 签名，可能显示未知发布者或 SmartScreen 提示；不要自动绕过系统安全策略。ARM64 包用于 Windows ARM64，不是 ARM32 或改名的 x64 程序；NSIS 安装器外壳可能仍是 x86，原生架构以解压主程序为准。Windows 需要 WebView2 runtime；保持 Tauri 默认 `downloadBootstrapper`，缺失时联网下载并执行 Microsoft Evergreen bootstrapper，由其选择与设备架构一致的运行时（ARM64 设备使用 ARM64）。未内嵌离线运行时，不承诺无网络首次安装；Windows 11 通常自带 WebView2，但仍须处理缺失情况。云端交叉编译/解包校验不是 Windows ARM64 安装或运行 smoke。

Linux 以 Ubuntu 22.04 为构建基线，不保证兼容更旧 glibc。AppImage 需执行权限，并可能需要系统 FUSE 支持；deb 需要发行版提供 WebKitGTK 4.1 等依赖。

本流程不发布 `latest.json`、updater 签名、镜像或上游下载链接。Tauri 配置必须保留 `createUpdaterArtifacts: false`，应用 owner 负责确认运行时代码也不再请求上游更新端点。不得为了发布自动生成/上传私钥、启用宽松认证或上传本机用户数据。Apple 公证、Windows 商业签名另行授权后单独设计。MIT LICENSE 及历史作者归属保持不变。

## 权限与独立仓库

当前仓库是从干净产品快照建立的独立历史，仅保留默认分支 `dev` 和发布分支 `release`。旧 fork 的所有历史保存在只读归档 [LumaGate-legacy](https://github.com/Restry/LumaGate-legacy)；不要向其推送、解除归档或删除历史。

维护者通过 SSH 推送。首次发布先推送 `dev` 并设置为默认分支，再将同一提交推送到 `release`。后续修改 workflow 也先更新默认 `dev`，再推进 `release`，避免标签/发布权限不识别新 workflow。

```bash
git push origin dev
gh repo edit Restry/LumaGate --default-branch dev
git push origin dev:release
```

不需要等待 dev CI 完成：release 会独立执行同一组轻量检查。不要强推或回退 release。可设置防删除/防强推保护，但不要求单人仓库额外审批或未经确认的必需 status checks。

如 SSH push 因 workflow scope 被拒，由维护者使用 `gh auth refresh --hostname github.com --scopes workflow` 完成官方授权流程；不复制 token 到 Git remote、命令日志或全局环境文件。

观察精确源码 SHA 的运行与实际下载，而不是只看 YAML：

```bash
gh run list --repo Restry/LumaGate --workflow release.yml --branch release --commit SOURCE_SHA
gh run watch RELEASE_RUN_ID --repo Restry/LumaGate --exit-status
gh release view TAG --repo Restry/LumaGate --json tagName,isDraft,isPrerelease,assets,url
gh api repos/Restry/LumaGate/git/ref/tags/TAG
gh release download TAG --repo Restry/LumaGate --dir /tmp/lumagate-verified-downloads
python3.13 scripts/releasing/release.py verify --version VERSION --sha SOURCE_SHA --input /tmp/lumagate-verified-downloads
```

annotated tag 的 ref 指向 tag object，继续 GET `/git/tags/OBJECT_SHA` 核对 `object.sha == SOURCE_SHA`。清单要求的资产必须全部下载并校验。Windows ARM64 安装器还需使用 `7z x`（macOS 可用 `7zz x`）解包，将其中主程序的 PE Machine `0xAA64`、大小、SHA-256 与 BUILD-INFO 回执核对，不能只检查安装器外壳。在 Mac 可使用 `hdiutil imageinfo` 和 `hdiutil verify` 检查 DMG，无需安装或替换运行中的应用。未在对应系统安装时，不声称 Windows/Linux 安装或运行已经验证。

默认 token 权限为 read。仅 reserve（创建 annotated tag/ref）和 publish（草稿及资产）job 请求 `contents: write`；编译、依赖安装及 PR 检查不写仓库。无 `secrets: inherit` 或 `pull_request_target`，checkout 不持久化凭据。缓存仅保存依赖下载，不含 target、安装器或本机状态。任何必需构建失败均阻止发布，不忽略失败、不上传模拟文件。

Node 版本沿用 `.node-version`。当前 Node 22.12.0 自带 Corepack 0.29.4，其旧 npm 公钥会使 pnpm 10.12.3 下载报 `Cannot find matching keyid`；CI 在两个构建入口先安装固定 `corepack@0.34.6`，再按 `packageManager` 执行 `corepack install`。此组合已在隔离目录实测，不禁用 Corepack integrity checks。

Windows hosted runner 的 Node 自带 Corepack 与 npm 全局 prefix 不同，单纯升级全局包仍可能解析到旧 shim。Windows 在 runner 临时目录安装固定 Corepack，以绝对 JS 路径启用和下载 pnpm，并将同一目录置于 PATH 首位；进入构建前打印路径并验证 Corepack 0.34.6 / pnpm 10.12.3。不禁用包签名检查、不依赖预装 Yarn、不改动开发者本机 npm 配置。

## 本地发布工具验证

无需安装应用依赖或编译应用；Python 3.11+：

```bash
python3.13 scripts/releasing/release.py check
python3.13 -m unittest discover -s scripts/releasing -p 'test_*.py' -v
actionlint .github/workflows/ci.yml .github/workflows/release.yml
```

注意：actionlint 1.7.12 尚不认识官方已支持的 `concurrency.queue`。该版本直接检查 release.yml 会报告这一处兼容性错误；不要因此删掉 `queue: max`。可另用 `-ignore 'unexpected key "queue" for "concurrency" section'` 检查其余语法，再单独按官方文档校验固定 group、`queue: max` 与 `cancel-in-progress: false`；这不等于声称未经筛选的 actionlint 全通过。

`stamp` 只应在临时构建 checkout 运行。测试使用临时目录，不访问 `~/.lumagate`、旧用户目录或 live gateway 端口。发布 API 命令还会核对仓库、分支、事件与 checkout SHA，不能作为本地随意发布脚本使用。

## 契约来源

- [GitHub concurrency / queue:max](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/control-workflow-concurrency)
- [GitHub hosted runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
- [Git references API](https://docs.github.com/en/rest/git/refs)、[Releases API](https://docs.github.com/en/rest/releases/releases)、[Release assets / digest](https://docs.github.com/en/rest/releases/assets)
- [Tauri distribution/versioning](https://v2.tauri.app/distribute/)、[macOS ad-hoc signing](https://v2.tauri.app/distribute/sign/macos/)、[Linux AppImage baseline](https://v2.tauri.app/distribute/appimage/)
- [Tauri Windows ARM64 / WebView2](https://v2.tauri.app/distribute/windows-installer/)、[Rust Windows MSVC 跨架构编译](https://doc.rust-lang.org/rustc/platform-support/windows-msvc.html)
- [Windows 2022 runner 预装 ARM64 工具](https://github.com/actions/runner-images/blob/main/images/windows/Windows2022-Readme.md)、[Microsoft WebView2 按设备架构安装](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)
- Action inputs 按对应 commit 的 `action.yml` 校验，不使用未经核实的第三方发布 action 参数。
