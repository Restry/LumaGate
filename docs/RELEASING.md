# LumaGate 发布流程

## 日常开发与下载

- `dev` 是日常开发分支和仓库默认分支。推送 `dev`、向 `dev`/`release` 提交 PR 不运行自动 CI，也不会发布。
- 合并前在本地运行 TypeScript 检查、当前 gateway UI 的 `pnpm test:manual`、四处版本与发布工具契约测试；命令见 CONTRIBUTING.md 及下方本地验证章节。GitHub Actions 仅负责正式发布构建、产物校验与发布，不再单独运行测试门禁。renderer 与 Rust 的真实编译由 release 的 Tauri 构建完成，保留原生 `beforeBuildCommand`。修改 Rust 手动网关逻辑时按需本地运行 `--lib manual::`。
- 将已完成变更从 `dev` 合入 `release` 并推送，触发 `LumaGate Release`。不需要手动打标签。
- 发布流程在同一个 workflow 中依次完成版本保留、五个目标构建任务、完整性校验、草稿上传、正式发布。不会依赖 `GITHUB_TOKEN` 创建标签后触发另一条 workflow。
- 用户最终从 <https://github.com/Restry/LumaGate/releases> 下载。Actions artifacts 仅用于任务交接，保留 14 天，不是最终下载地址。

## 版本与源码关系

基础版本来自 `package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 和 Cargo.lock 中的本地应用包，四处必须一致且为稳定 `MAJOR.MINOR.PATCH`。

独立标签命名空间为 `lumagate-vX.Y.Z`，不使用/覆盖上游历史 `v*` 标签：

1. 首次发布使用经过校验的基础版本；LumaGate 首发集成的四处基础版本为 `3.24.0`。
2. 同一 major/minor 系列后续新 SHA：取基础版本与已有保留版本下一 patch 的较大值。按数字比较，不按字符串比较。
3. 显式 minor/major 发布：在 `dev` 将四处基础版本同步到新的 `X.Y.0` 或 `X.0.0`，提交并合入 `release`。只写 conventional commit 的 `feat!:` 不会自动改版本。不要将基础版本回退到已经发布过的更低 major/minor。
4. 发布条件与基础版本校验通过后，用 annotated tag 保留版本，标签直接指向触发事件的完整源 SHA，annotation 记录版本与 SHA。标签不是 Release。构建失败会留下保留标签，可能出现版本空号；不得删标签回收版本。
5. 同一 SHA 的重跑/手动重试复用原版本和原标签，包括较旧失败运行的重跑。成功发布后不重新构建、不覆盖资产，而是下载原始发布资产核对 SHA、版本、清单和校验和。
6. 仅在 GitHub runner 的构建 checkout 修改上述四处版本；不提交 bump commit，不修改依赖版本，不形成 CI 循环。Rust `CARGO_PKG_VERSION` 与 Tauri runtime/安装器版本因此一致。`BUILD-INFO.json` 将版本映射回精确源码 SHA、目标架构及各平台文件摘要。源码标签中的 manifest 仍是基础版本；复现时须先执行 `stamp`。
7. 为兼容 Windows installer，major/minor 最大 255、patch 最大 65535；到界限必须显式升 minor/major，不能自动溢出。

整个发布 workflow 使用固定 concurrency group、`cancel-in-progress: false`、`queue: max`。GitHub 最多保留 100 个 pending 运行；超过容量的运行可能取消，需要在原运行上重试。按进入等待队列的顺序处理，不承诺按 push 时间排序。旧版本补发不会将 Latest 从更高版本移回去。

已保留的 annotated tag 是发布源码真值；创建 Release 不再传 `target_commitish`，在创建前与公开前均核验 tag→SHA。GitHub 会检查显式历史 target 与默认分支的 workflow 差异，GITHUB_TOKEN 无法取得 workflows:write；旧运行 37469856404 因此在创建 Release 返回 403。不得通过上传个人广权限 token 绕过，也不重写原标签。

## 必需资产：安装包、签名更新包与校验文件

下表以 `VERSION` 表示分配后的版本。schema 3 从平台清单推导完整资产集合，目前为 **16 个上传资产**；不是硬编码数量门禁。GitHub 自动生成的源码 zip/tar 不计入。

| Runner | Rust target | 必需文件 |
| --- | --- | --- |
| `macos-15` | `aarch64-apple-darwin` | `LumaGate-VERSION-macos-arm64.dmg`、`.app.tar.gz`、`.app.tar.gz.sig` |
| `macos-15-intel` | `x86_64-apple-darwin` | `LumaGate-VERSION-macos-x64.dmg`、`.app.tar.gz`、`.app.tar.gz.sig` |
| `windows-2022` | `x86_64-pc-windows-msvc` | `LumaGate-VERSION-windows-x64.exe`、`.exe.sig` |
| `windows-2022`（交叉编译） | `aarch64-pc-windows-msvc` | `LumaGate-VERSION-windows-arm64.exe`、`.exe.sig` |
| `ubuntu-22.04` | `x86_64-unknown-linux-gnu` | `LumaGate-VERSION-linux-x64.AppImage`、`.AppImage.sig`、`.deb` |
| 汇总 | — | `latest.json`、`BUILD-INFO.json`、`SHA256SUMS` |

使用 Tauri 配置及目标 bundle 目录发现产物，不硬编码旧 `cc-switch` 可执行文件名。每种格式必须恰好匹配一个文件，缺失、重复、跨 SHA/版本/平台混装、未知额外文件、空文件、摘要不符均阻止发布。macOS 检查 app 内版本、Mach-O 架构、结构性 codesign 验证及 DMG 校验；Windows 用 runner 预装的 7-Zip 解压 NSIS，检查其中主程序的 PE Machine（x64 `0x8664`、ARM64 `0xAA64`）、ProductVersion/ProductName，并核对其与构建主程序的摘要/大小一致；Linux 检查 deb 版本/架构和 AppImage ELF 架构。检查不会启动应用或接触用户数据。

macOS 必须保留 `app,dmg` 构建参数，供 Tauri 同时生成 `.app.tar.gz` 和 `.sig`，并验证 `.app` 的版本、架构与 ad-hoc 签名；不要恢复仅构建 dmg 的旧错误。

原始输出只上传本次 workflow 的平台 artifact。汇总不合并同名目录，先检查每个平台自己的 `build.json`，再生成所有安装包及 `BUILD-INFO.json` 的 `SHA256SUMS`（不包含自身）。上传草稿后再次检查 GitHub 返回的完整资产名称、推导数量、大小、上传状态及 SHA256 digest，最后才将 `draft=false`、`prerelease=false`。任何必需平台失败都不能发布残缺版本。

新 `BUILD-INFO.json` 使用 `schema: 3`，保留五目标及 Windows 解包回执，并记录更新公钥指纹。无 schema 的原契约仅适用于 `3.24.2` 及以前；schema 2 仅适用于 `3.24.6` 及以前。历史公共资产与标签不重写。每个平台在 stage 时验证真实 minisign 签名，汇总再次验签后生成静态 `latest.json`；其中 signature 为 `.sig` 的完整内容，URL 为最终公开资产路径。目标键为 darwin-aarch64/x86_64、windows-aarch64/x86_64、linux-x86_64。

Windows ARM64 使用已有 `windows-2022` 上的 `Microsoft.VisualStudio.Component.VC.Tools.ARM64` 与 `rustup target add aarch64-pc-windows-msvc`。正式编译前用同目标编译/链接一个最小 Rust 程序，尽早暴露工具链问题。Rust 与 Tauri 官方支持 Windows 主机跨架构编译；不依赖本地 VM。GitHub 也提供 `windows-11-arm`，本流程为复用已验证的 Node/Corepack/VS2022 环境而不切换 runner。

## 失败与重试

```bash
# 选定精确源 SHA 的运行，记录 RUN_ID，不要误看另一个 push。
gh run list --repo Restry/LumaGate --workflow release.yml --branch release --commit SOURCE_SHA
gh run view RUN_ID --repo Restry/LumaGate --log-failed
gh run rerun RUN_ID --repo Restry/LumaGate --failed
gh run watch RUN_ID --repo Restry/LumaGate --exit-status
```

- 发布条件或基础版本校验失败：尚未保留版本，无 Release。
- 矩阵失败：保留标签仍在，成功的平台 artifact 可供同一运行的 failed-jobs 重跑使用；不生成 Release。
- 上传中断：仅留下不可公开下载的草稿，不更新 Latest。下次发布会验证草稿的 tag/SHA 归属，删除该未发布草稿并重新上传完整资产；永远不删除标签、公共 Release 或公共资产。
- 上传失败后的草稿无需手动清理；保留可供诊断。若中断恰好发生于公开成功后，重跑读取公共发布状态，不盲目删除。网络/API 错误不伪装成“没找到”。
- 14 天后 artifacts 已过期：使用 `gh run rerun RUN_ID --repo Restry/LumaGate` 重跑整次运行。不要只重跑 publisher。
- 同一版本不承诺跨工具链重新构建的二进制字节完全相同，所以公共版本只验证原始下载，不覆盖重建文件。
- `workflow_dispatch` 仅允许 `release` ref；在 `dev`、标签或其他分支选择运行会被 guard 拦截。手动运行当前 release：`gh workflow run release.yml --ref release --repo Restry/LumaGate`。恢复旧 SHA 应重跑原 RUN_ID，不要倒退/强推 release 分支。
- GitHub API/权限、runner 可用性、包源网络、构建失败必须修复并重跑；不能删掉失败平台冒充成功。

## 安装与签名边界

必须配置项目专用 `TAURI_SIGNING_PRIVATE_KEY` 与 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 两个 GitHub Actions Secrets，仅在受信任 release 构建步骤注入。Tauri 签名强制验证，独立于平台代码签名。macOS 使用 `-` 做 ad-hoc 签名，**不是 Developer ID、公证或 Gatekeeper 认可**；首次安装可能需要系统单应用允许流程，不全局关闭 Gatekeeper、不清除 quarantine。

Windows 安装器未做 Authenticode 签名，可能显示未知发布者或 SmartScreen 提示；不要自动绕过系统安全策略。ARM64 包用于 Windows ARM64，不是 ARM32 或改名的 x64 程序；NSIS 安装器外壳可能仍是 x86，原生架构以解压主程序为准。Windows 需要 WebView2 runtime；保持 Tauri 默认 `downloadBootstrapper`，缺失时联网下载并执行 Microsoft Evergreen bootstrapper，由其选择与设备架构一致的运行时（ARM64 设备使用 ARM64）。未内嵌离线运行时，不承诺无网络首次安装；Windows 11 通常自带 WebView2，但仍须处理缺失情况。云端交叉编译/解包校验不是 Windows ARM64 安装或运行 smoke。

Linux 以 Ubuntu 22.04 为构建基线，不保证兼容更旧 glibc。AppImage 需执行权限，并可能需要系统 FUSE 支持；deb 需要发行版提供 WebKitGTK 4.1 等依赖。

只使用 `https://github.com/Restry/LumaGate/releases/latest/download/latest.json`；无上游 CC Switch 或测试端点。`createUpdaterArtifacts: true`，公钥固定在客户端。全部平台与验签完成前始终保持 draft；最后只公开一次。不得上传私钥、用户数据、Vault 主身份或绕过 HTTPS/验签。Linux 应用内安装仅支持 AppImage，deb 明确走包管理器/手动升级。

### 更新密钥保管

Vault 句柄为 `lumagate/TAURI_SIGNING_PRIVATE_KEY` 和 `lumagate/TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。维护机另有 `~/.local/share/lumagate/updater-signing/` 私有备份（目录 0700、文件 0600）。公钥 SHA-256 为 `282e6ddadcdc1899364538d80f9d1a8b96d2032df1a7b31b5ba5511a50fdff9b`。不得随意重生成或轮换；丢失对应私钥后，现有客户端无法接受新签名。

恢复时在不回显内容的子进程中从 Vault 读取，验证本地 byte hash，再用 stdin 传给 `gh secret set`；API 只能确认 Secret 名称，必须通过实际构建产物验签确认密钥正确。Vault 写入后核对加密文件的提交和后续回读；遇并发同步丢失仅恢复自己的加密条目，不全局 reset。生成命令会继承 `CI=true` 并跳过密码提示，交互生成必须显式移除 CI；必须实际签名验证密码后才能认为托管完成。

### 应用恢复边界

首次引导需手动覆盖安装启用 updater 的版本，先备份 `~/.lumagate`、自身 WebKit 偏好和客户端配置。之后应用检查、确认、下载、验签、原生排空、安装、重启自动完成。后端使用原子准入屏障，不依赖 UI active 数；HTTP 响应及 Hyper 尾帧排空后刷新日志。Windows 在 plugin install 调用前排空，并通过 on_before_exit 处理退出清理。

下载失败/签名不符不触碰旧包；等待时取消恢复接收请求。安装失败恢复原监听（能恢复时），明确不保证自动回滚；必要时用官方完整安装包覆盖，勿删除数据目录。macOS 检查普通用户安装目录权限，权限不足不提供 sudo 绕过。一次性重启标记只恢复升级前真正运行的地址和端口，不改变保存的设置，不把普通启动变成自动开网。

## 权限与独立仓库

当前仓库是从干净产品快照建立的独立历史，仅保留默认分支 `dev` 和发布分支 `release`。旧 fork 的所有历史保存在只读归档 [LumaGate-legacy](https://github.com/Restry/LumaGate-legacy)；不要向其推送、解除归档或删除历史。

维护者通过 SSH 推送。首次发布先推送 `dev` 并设置为默认分支，再将同一提交推送到 `release`。后续修改 workflow 也先更新默认 `dev`，再推进 `release`，避免标签/发布权限不识别新 workflow。

```bash
git push origin dev
gh repo edit Restry/LumaGate --default-branch dev
git push origin dev:release
```

检查和测试须在合并前本地完成；dev/PR 不运行自动 CI，release 不再调用可复用 CI 门禁。不要强推或回退 release。可设置防删除/防强推保护，但不要求单人仓库额外审批或未经确认的必需 status checks。

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

默认 token 权限为 read。仅 reserve（创建 annotated tag/ref）和 publish（草稿及资产）job 请求 `contents: write`；编译与依赖安装不写仓库。无 `secrets: inherit` 或 `pull_request_target`，checkout 不持久化凭据。缓存仅保存依赖下载，不含 target、安装器或本机状态。任何必需构建失败均阻止发布，不忽略失败、不上传模拟文件。

Node 版本沿用 `.node-version`。当前 Node 22.12.0 自带 Corepack 0.29.4，其旧 npm 公钥会使 pnpm 10.12.3 下载报 `Cannot find matching keyid`；发布流程在 Windows/Unix 构建入口先安装固定 `corepack@0.34.6`，再按 `packageManager` 执行 `corepack install`。此组合已在隔离目录实测，不禁用 Corepack integrity checks。

Windows hosted runner 的 Node 自带 Corepack 与 npm 全局 prefix 不同，单纯升级全局包仍可能解析到旧 shim。Windows 在 runner 临时目录安装固定 Corepack，以绝对 JS 路径启用和下载 pnpm，并将同一目录置于 PATH 首位；进入构建前打印路径并验证 Corepack 0.34.6 / pnpm 10.12.3。不禁用包签名检查、不依赖预装 Yarn、不改动开发者本机 npm 配置。

## 本地发布工具验证

无需安装应用依赖或编译应用；Python 3.11+：

```bash
python3.13 -m venv /tmp/lumagate-release-check
/tmp/lumagate-release-check/bin/pip install cryptography==46.0.3
# 以下 Python 命令使用该隔离环境，避免修改用户已有依赖。
/tmp/lumagate-release-check/bin/python scripts/releasing/release.py check
/tmp/lumagate-release-check/bin/python -m unittest discover -s scripts/releasing -p 'test_*.py' -v
actionlint .github/workflows/release.yml
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
