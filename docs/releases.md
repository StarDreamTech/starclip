# 软件升级与发布

星笺使用 Tauri 2。程序安装位置与数据目录分开，正常升级替换程序，继续使用原数据。不要先卸载再使用清理工具删除数据；自定义数据目录以设置页显示的位置为准。已有保留天数和条数规则仍会正常运行，升级不会将用户偏好改回默认值。

## 用户升级

- **已有 0.1.0**：该版本没有更新器。升级到 0.2.4 时，退出星笺；macOS 将新的 `Starclip.app` 覆盖到原安装位置，Windows 运行新版安装包。保留数据目录和应用标识 `com.starclip.desktop`。
- **0.2.0 及以后**：在设置中检查更新、阅读发布说明，再选择「下载更新」。HTTPS 下载并通过应用内置公钥及签名版本校验后，弹出「安装并重启」；可选「稍后再说」继续使用，已下载包保留到本次应用退出或再次手动检查更新。只有确认安装才保存当前数据恢复点和旧应用副本；0.2.3 的 macOS 重启流程会先完成正常退出清理，Windows 由安装器退出、替换并重新打开应用；从旧版升级的重启边界见下一条。
- **macOS 0.2.2 升级到 0.2.3**：旧版可能在安装完成后退出而未自动重新打开。请从「应用程序」重新打开星笺一次，不必卸载或删除数据；本次真实旧版验收已确认新包替换、手动重开和数据保留。0.2.3 中的修复不能追溯改变旧进程的重启行为，该次验收不计为自动重启通过。
- 更新服务不可达、尚无公开版本、签名不符或恢复点创建失败都会报告错误；不会把失败显示成已完成。GitHub 上的草稿对普通客户端不可见。
- 浏览器预览不能安装桌面更新。当前更新平台为 Apple Silicon macOS 和 Windows x64；Intel Mac、Linux、其他 CPU 架构没有发布产物。

### 更新提醒与版本通道（0.2.3）

「偏好设置 → 帮助 → 软件更新与恢复」中的自动检查默认开启，开关即时保存。应用启动约 20 秒后尝试检查，此后距上次检查至少 24 小时才再次自动联网；失败、手动检查和重启都会保留检查时间，避免反复请求。可随时关闭自动检查或主动手动检查。只向 GitHub 请求更新信息，不上传剪贴内容。

新版提示出现在主界面空闲时，支持查看更新和忽略当前版本；不会唤起已关闭的主窗、抢焦点或发系统通知，自动检查失败也不会弹出提示。下载安装仍需用户操作，已下载待安装包不会被自动检查清除。

当前正式通道只接受 `X.Y.Z` 稳定版本，排除预发布标记及 `alpha` / `beta` / `rc` 等版本；Homebrew 同步仍只跟随公开 latest 稳定版。预览通道尚未开放。LTS 表示对某个版本提供长期维护，不等于稳定版的另一个名称；项目目前没有承诺维护期限或独立 LTS 分支，因此界面使用「稳定版」。

数据恢复与重新安装旧程序是两件事。恢复点选择、恢复到新目录和失败处理以设置中的说明为准；详见 [产品范围](product.md)。

## Homebrew 安装源

星笺的自有 Tap 是 [cpython666/homebrew-tap](https://github.com/cpython666/homebrew-tap)，不需要额外 Homebrew 账号。安装描述位于该仓库的 `Casks/starclip.rb`，安装包仍由本软件仓库的 GitHub Release 提供。

```sh
brew install --cask cpython666/tap/starclip
# 维护者发布并更新 Tap 后，先退出应用再升级：
brew update
brew upgrade --cask cpython666/tap/starclip
```

[0.2.4 已公开为最新稳定版](https://github.com/cpython666/starclip/releases/tag/v0.2.4)；[Homebrew 同步](https://github.com/cpython666/starclip/actions/runs/41580841113) 已完成，当前 Cask 为 **0.2.4**。流水线核对公开 DMG、应用签名 / 版本 / 架构及 Homebrew 在线审计，并通过[机器人提交](https://github.com/cpython666/homebrew-tap/commit/2c961507eea106de52b9e667ba029cc39826ab0f)更新 Tap；真实远端 Cask 的 SHA-256 与公开 DMG 一致。历史 alpha.1 / alpha.2 附件保留；0.2.1 因发布门禁失败未公开。程序具有 Tauri 更新签名，macOS 为 ad-hoc，仍无 Apple Developer 公证或 Windows 发行者证书，本地验签不等于 Gatekeeper 首启验收。

如果较新 Homebrew 提示该 Cask 尚未受信任，执行 `brew trust --cask cpython666/tap/starclip` 后重试；这只信任星笺这一项，不需要信任整个 Tap。

已有手动安装或较新本地开发版时，不用 `--force` 覆盖或降级。此 Cask 没有 `zap` 或删除数据脚本，普通升级 / 卸载只管理程序文件；程序内部的数据迁移边界仍由相应版本决定。自有 Tap 不代表 Homebrew 官方主仓收录；WinGet 尚未接入。

正式 Release 发布后的 Tap 同步由 `.github/workflows/homebrew.yml` 承担：只处理公开、非预发布且为 latest 的 `vX.Y.Z`，重新下载 DMG 核对摘要、签名、应用身份 / 版本 / 架构并完成 Homebrew 检查后，才更新 Cask 的版本与 SHA-256。失败保留原 Cask；旧任务不得降级，同版本换摘要会被拒绝。0.2.2 已完成首次完整自动同步；没有把历史 alpha 改标为稳定版或覆盖其附件。配置、重试与预演见 [Homebrew 发布自动化](homebrew-release.md)。0.2.4 已完成双平台签名、全新 / 旧版覆盖安装、真实公开 0.2.3 应用内升级和匿名公开附件验收；详细证据见下方「当前验证边界」。

## 版本与发布说明

稳定版本格式为 `x.y.z`，发布 tag 必须是 `vX.Y.Z`。版本统一保存在 `package.json`、`package-lock.json` 两处、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 和 `src-tauri/tauri.conf.json`。

```sh
npm run version:check
npm run version:bump -- X.Y.Z
npm run version:check -- vX.Y.Z
```

将示例中的 `X.Y.Z` 替换为要发布的新版本。递增脚本先验证原版本一致，再同步这些文件；不修改依赖版本。不要只手改 Tauri 配置，或直接使用只更新 npm 文件的 `npm version`。每次发版同时在 [CHANGELOG.md](../CHANGELOG.md) 增加对应的 `## [X.Y.Z]` 条目。发布脚本从该条目生成客户端说明和 Release 正文。

`.github/workflows/verify.yml` 在分支提交、PR 和手动触发时运行版本校验、Node 测试、Web 构建、Rust core 测试与两个桌面平台构建；构建后复用 `installer-smoke.yml` 检查 fresh / 0.1.0 覆盖升级及固定数据保留。Windows 历史基线是固定成功 Actions artifact，若过期须更新为可核实的旧版安装包，不能静默跳过。普通验证不生成签名更新包、不需要发布私钥。

## 一次性配置

1. 使用能公开下载附件的 GitHub 仓库。应用当前稳定更新源固定为 `https://github.com/cpython666/starclip/releases/latest/download/latest.json`；更新端点与公钥位于 `src-tauri/tauri.conf.json`。迁移仓库前须保留旧地址的访问路径，或先通过旧地址发布包含新地址的过渡版本。
2. 保管与内置公钥对应的 Tauri 更新私钥，在仓库外安全备份。GitHub 仓库 → Settings → Secrets and variables → Actions 增加 `TAURI_SIGNING_PRIVATE_KEY`（完整私钥文件内容）；有密码时另设 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，无密码时留空。不要把私钥、密码或真实数据加入 Git，不要在命令输出中打印。
3. GitHub Actions 需要正常运行权限；只有最终创建草稿的 job 请求 `contents: write`。无需个人访问令牌，使用本次 workflow 的 `GITHUB_TOKEN`。建议限制发布 tag 的创建权限。
4. 如需 macOS 正式签名及公证，可配置 `APPLE_SIGNING_IDENTITY`、`APPLE_CERTIFICATE`（导出的 `.p12` Base64 内容）、`APPLE_CERTIFICATE_PASSWORD`，以及 `APPLE_ID`、`APPLE_PASSWORD`（应用专用密码）、`APPLE_TEAM_ID`。工作流已将这些可选 secrets 交给 Tauri。未配置时使用 `-` ad-hoc 身份；这不等于 Apple Developer 签名或公证。Windows Authenticode 尚未接入。
5. Homebrew 跨仓同步使用另一个专用凭据 `HOMEBREW_TAP_DEPLOY_KEY`，只授权 `cpython666/homebrew-tap` 写入，与 Tauri 更新签名私钥分开。只有验证通过后的 Tap job 使用它；不用本机 GitHub 登录令牌。详见 [配置说明](homebrew-release.md)。

初次建立其他产品/全新发布身份时可用以下命令生成密钥；本项目已有对应内置公钥的密钥，不要每次发版重新生成：

```sh
# 请替换为仓库之外的私有目录，避免覆盖已有密钥。
npm run tauri -- signer generate -w /private/path/starclip-updater.key
```

公钥变更需要旧密钥签署的过渡版本。丢失旧私钥将无法继续向已有客户端发送可接受的更新，需要用户手动安装新的信任版本。更新签名只证明更新包来源，和操作系统的发行者证书、公证是不同的校验。

官方依据：[Tauri Updater](https://v2.tauri.app/plugin/updater/)、[macOS 签名与公证](https://v2.tauri.app/distribute/sign/macos/)、[Windows 签名](https://v2.tauri.app/distribute/sign/windows/)。

## 每次发布

1. 更新版本和 CHANGELOG，完成检查并审核代码。在已确认的提交上创建并推送匹配 tag，例如 `v0.2.3`；也可以在 Actions 的 **Build signed release draft** 手动运行，输入已经存在的 tag。发布脚本不会创建或移动 tag。
2. 等待完整流水线成功：版本/tag/说明检查 → Node 与 Rust core 测试 → macOS app/DMG、Windows NSIS → 两平台原生安装 / 固定数据覆盖升级 → updater 签名核验 → 草稿附件上传。
3. 汇总阶段使用 `minisign` 验证产物的真实签名，并检查其密钥与应用公钥、签名绑定版本与发布版本一致；要求两个平台和 macOS DMG 齐全。签名文件缺失、产物重复、版本不一致或验签失败均停止，不更新公开的 `latest.json`。
4. 下载草稿产物，在隔离测试数据上验收安装与升级。至少检查：旧版数据和偏好保留、图片及星伴存档、恢复点失败会阻止安装、正常恢复和不兼容 schema 拒绝写入、签名损坏拒绝安装，以及每个平台安装后能重启。不要读取真实剪贴板来做测试。
5. 全部 job 成功且实机验收通过后，在 GitHub Release 页面人工审核并 **Publish release**，选择设为最新正式版本；不要标记为 prerelease。发布会让客户端的稳定地址指向这一版。不要在工作流仍上传附件时提前发布。
6. **Update Homebrew after release** 会验证公开 DMG 和 Cask，再自动推送 Tap。检查此 workflow 结果；若网络或 Tap 推送冲突失败，可在 Actions 输入相同 tag 重试，默认 `dry_run=true` 只预演，确认要同步时设为 false。此同步的包验证不替代上一步的安装与升级验收。

工作流从不自动公开发布，也不覆盖已公开的同 tag Release；需要修复时递增版本。草稿上传中途失败只影响草稿，可重跑同一 tag；在 workflow 成功前不要发布。普通 CI 成功、签名核验成功或草稿已生成，都不能代替 Windows/macOS 的原生升级验收。

发布附件：

| 附件 | 用途 |
| --- | --- |
| `Starclip_X.Y.Z_aarch64.dmg` | macOS 手动安装 |
| `Starclip_X.Y.Z_aarch64.app.tar.gz` 和 `.sig` | macOS Tauri 更新器 |
| `Starclip_X.Y.Z_x64-setup.exe` 和 `.sig` | Windows NSIS 安装及 Tauri 更新器 |
| `latest.json` | 版本、说明、发布时间、两平台 HTTPS 下载地址及签名内容 |
| `SHA256SUMS`、`release-notes.md` | 人工核对校验值与更新说明 |

更新清单使用 Tauri 2 的 `darwin-aarch64` / `windows-x86_64` 键。`.sig` 的完整内容写入 `signature` 字段；不填写签名文件 URL。客户端同时启用 `requireSignedVersion`，拒绝更新清单版本与签名绑定版本不一致的包。全部附件也作为 `complete-release-X.Y.Z` Actions artifact 保留，保留时长由仓库 Actions 设置决定。

## 故障与回退

- **下载/签名/备份失败**：保留现有程序并显示错误，修复网络、空间或发布产物后重试。不要通过关闭签名校验解决。
- **新版本发布后发现问题**：先停止推广该版本，修复后发布更高版本。把 GitHub 的 latest 指回旧版本不会使已升级客户端自动降级，默认更新比较器只接受更高版本。
- **需要恢复数据**：使用升级前恢复点恢复到新目录并切换，保留原目录以便核对。恢复后将不包含恢复点之后的新记录；这一步不会替换程序文件。
- **确需退回旧程序**：同时准备与旧程序兼容的数据库副本，退出所有星笺进程后操作；不能让旧版直接打开结构更高的数据。0.2.0 起会拒绝高于自身支持范围的 schema，但 0.1.0 本身没有这道保护，不能依赖它识别新数据库。优先使用向前修复版本。
- **迁移失败**：启动保持错误信息和恢复入口，数据库迁移事务会撤回，迁移前恢复点仍保留。不应绕过错误新建空库，从而掩盖旧数据位置。

用户数据、恢复点、更新私钥均不得进入 Actions artifact 或 Release 附件。当前自动汇总脚本只接收固定命名的程序安装包和签名；不打包工作目录或数据目录。

## 当前验证边界

0.2.4 已于 2026-11-03 按用户授权正式发布，tag 固定 `6d83640`，[签名构建 41579229975](https://github.com/cpython666/starclip/actions/runs/41579229975) 8/8；真实公开 0.2.3 → 此精确签名目标的 [macOS 41580453911](https://github.com/cpython666/starclip/actions/runs/41580453911) / [Windows 41580457922](https://github.com/cpython666/starclip/actions/runs/41580457922) 均自动重启及原创数据 / PNG / 偏好 / 成长 / 暂停 / 统计关闭与恢复副本保留通过。原生 UI 使用一次性 runner 临时可信 HTTPS 服务，保留正式 URL 与签名包原始字节；匿名公开 8 附件及 latest 的完整字节另已核对一致，不扩大为公网 CDN 原生更新 GUI 验收。本机正式应用未替换。完整摘要、清理与证据见 [0.2.4 发布与升级](verification.md#024-正式发布与原生升级验收)。

本轮 updater workflow 默认真实 `public-0.2.3`，包大小 / SHA / 内部版本由 `scripts/updater_baselines.py` 固定；0.2.3 必须自动重启，显式旧 0.2.2 仍可记录历史手动恢复，macOS source-comparison 实验保留并单独标明。下次发布应先更新并核对当时的真实公开基线，不以比较版本实验替代。

0.2.2 发布时记录的 11 项发布和版本脚本夹具测试，覆盖版本不一致、错误 tag、递增、缺失/重复产物、错误签名结构、密钥与绑定版本、不完整平台、验签失败和更新说明提取。已另用临时原创文件通过官方 Tauri signer → minisign 0.12 → 清单汇总的真实签名验证，并确认篡改文件被拒绝；两个 workflow 通过 actionlint 1.7.12 静态检查。原签名密钥已配置到 GitHub 加密 secret；0.2.2 的两平台签名构建、草稿上传、原生安装与覆盖升级已通过实际工作流，公开更新清单和下载包也已核验。这一阶段的原生安装冒烟只检查临时 runner 的启动和固定数据升级，不能单独证明完整应用内下载 / 弹窗 / 自替换链路。

2026-10-29 Windows 验收使用产品提交 `7fbbae5` 的完整源码，将更新比较版本设为 0.2.1，经过真实 GUI 下载公开 0.2.2、选择稍后、再次确认安装、替换 EXE 并自动启动新进程；暂停偏好、原创固定记录 / 分组及更新前恢复点、旧 EXE 备份均核对通过。证据为一次性实验提交 `2b148e9` 的 [run 40821236132](https://github.com/cpython666/starclip/actions/runs/40821236132)。随后 macOS 提交 `22480ca20defb4abe7507a80d527dffca96d676a` 的 [run 40862701839](https://github.com/cpython666/starclip/actions/runs/40862701839) 也完成检查、签名下载、稍后、确认后快照 / 旧 app 备份、应用替换与自动重启；新旧 PID 不同，目标摘要匹配公开包，固定数据与暂停状态保留。macOS 测试在全新 runner 的空白正式命名空间运行，不访问本机正式安装或数据。

上述两平台历史实验只将所列源码的 Tauri / 打包比较版本覆盖为 0.2.1，目标固定为公开签名 0.2.2；不是历史旧 0.2.1 源码二进制升级，也不用于替代下面独立完成的 0.2.3 签名目标验收。Windows 更新通过不扩大为统计联网或字体 GUI 已验收。历史详细步骤见 [macOS 完整链路](verification.md#macos-应用内更新完整链路) / [Windows 完整链路](verification.md#windows-应用内更新完整链路)。

0.2.3 于 2026-10-29 经用户授权[正式发布](https://github.com/cpython666/starclip/releases/tag/v0.2.3)。[签名发布运行](https://github.com/cpython666/starclip/actions/runs/40882819215) 完成两平台构建、安装包与固定数据覆盖升级校验、签名验证及完整附件汇总。本次目标包另经过真实签名正例和改动包字节后拒绝的负例验证。真实公开 0.2.2 → 签名 0.2.3 的 [Windows 运行](https://github.com/cpython666/starclip/actions/runs/40891766297) 自动重启通过；[macOS 运行](https://github.com/cpython666/starclip/actions/runs/40895064653) 的结果为 `passed-with-manual-recovery`：目标程序已替换，旧版未自动重新打开，手动启动后暂停状态、固定文本 / PNG、分组、成长和偏好均保留，快照及旧应用备份也通过核对。两平台使用一次性 runner 上的临时受信任 HTTPS 服务，保留生产 URL 和签名包原始字节；该验收不证明公开 GitHub / CDN 下载链路，macOS 的手动重开也不能写成自动重启通过。临时 HTTPS 清理已验收，本机正式安装仍为 0.2.2，未替换。证据索引见 [0.2.3 正式发布与原生升级验收](verification.md#023-正式发布与原生升级验收)。
