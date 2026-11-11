# 发布后自动更新 Homebrew

`.github/workflows/homebrew.yml` 监听 GitHub Release 的 `published` 事件。维护者发布正式版本并设为 latest 后，工作流验证公开的 macOS 包，再把同一份 `Casks/starclip.rb` 提交到两个 Tap：个人仓库 `cpython666/homebrew-tap` 与组织仓库 `StarDreamTech/homebrew-tap`。它不创建 Release、不移动 tag、不公开草稿，也不重新构建或启动下载的应用。Cask 的 `url` 固定指向 `https://github.com/StarDreamTech/starclip` 的 Releases；0.2.6 起 `homepage` 改为 `https://stardreamtech.github.io/projects/starclip/`。两者都由 `scripts/homebrew-release.mjs` 的 `renderCask` 生成；仓库里没有单独的 `homebrew-cask.mjs`。已发布的 0.2.5 Cask 仍把 homepage 指向 GitHub 仓库页，要等下次正式 Release 同步后才会改成介绍页。

## 发布约定

1. 完成对应版本的测试、构建和安装 / 升级验收，再发布稳定 tag `vX.Y.Z`。tag 必须包含这份工作流；旧 tag 可用下面的手动运行入口。
2. Release 必须公开、不是 prerelease，且是 GitHub API 返回的最新稳定版本。alpha / beta 预览或较旧版本不会改动 Tap。
3. 上传 `Starclip_X.Y.Z_aarch64.dmg` 和 `SHA256SUMS`。校验文件使用标准格式：每行 64 位小写 SHA-256、两个空格、文件名；同一 DMG 只能有一个条目。Release 中的对应附件也必须唯一、非空且上传完成。
4. 再发布 Release。若发布时附件尚未齐全，工作流失败并保留原 Cask，补齐后手动重跑；不要让用户看到仍在上传的正式版本。

目前自动同步只支持 Apple Silicon macOS、固定应用 `Starclip.app` / `com.starclip.desktop`。发布版应用内部版本必须与 tag 一致，不能把内部版本仍为 `0.1.0` 的旧包改名成 `0.2.0`。

## 自动检查与写入边界

校验不启动应用，使用 `macos-15-intel` 并在 `brew audit` / `fetch` 显式指定 `--arch=arm`；仍通过 `lipo` 要求实际 app 为 arm64，Cask 仍仅支持 Apple Silicon。2026-11-11 首轮 ARM runner 因 GitHub 容量不足未取得任务，现采用该检查方式；它不替代 ARM 原生升级验收。

只读验证 job 从公开 Release 下载文件，重新计算 DMG 的 SHA-256，核对 `SHA256SUMS`、附件大小和可用时的 GitHub digest；随后 `hdiutil verify`、只读挂载、`codesign --verify --deep --strict`、核对应用版本 / 标识 / arm64。应用不会被启动。ad-hoc 签名通过不等于 Apple Developer 公证或 Gatekeeper 首启验收。

生成的 Cask 使用固定仓库和带版本的 URL，写入实际下载字节的 SHA-256；临时 runner 先更新 Homebrew 并输出版本，避免旧镜像的格式与安装包审计规则不一致。生成固定 Cask 且 `brew style --cask` 通过后，只对 `cpython666/tap/starclip` 执行 `brew trust --cask`，再运行 `brew audit --cask --strict --online` 和 `brew fetch --cask`。审计仍借用个人 Tap 的本地检出路径，被审计的是同一份 Ruby 文件，随后原样写入两个远端 Tap。该名称信任仅存在于本次 runner，不信任整个 Tap、不安装或启动应用；生成后不再修改被审计文件。没有 `zap`、删除用户数据脚本或绕过系统隔离的步骤；不设置 `auto_updates`。用户可按安装来源执行 `brew upgrade --cask cpython666/tap/starclip` 或 `brew upgrade --cask stardreamtech/tap/starclip`。

全部检查通过后，两个独立写入 job 才各自取得对应 Tap 凭据，重新查询 Release 是否仍公开、仍为 latest、附件身份是否未改变，再生成 Cask 并提交。`update-tap` 与 `update-org-tap` 都只依赖验证 job，互不阻塞。所有版本共用串行队列，同时按数字版本检查，避免旧任务覆盖新版本。同版本同摘要无操作，同版本换摘要拒绝；Git push 不使用 force，Tap 同时被别人修改时宁可失败重跑。组织 Tap 里还有其他产品的 Formula / Cask，提交只暂存 `Casks/starclip.rb`。

这套同步检查证明下载文件与 Cask 一致，不能代替源代码测试、双平台发布构建成功、Windows / macOS 的安装和升级验收。维护者仍应完成发布前验收。

## 一次性凭据

仓库自己的 `GITHUB_TOKEN` 只负责读取 Release，不能直接写另一个仓库。两个 Tap 各用一把专用 Ed25519 deploy key，互不代替：

| Tap | 安装命令 | 公钥所在仓库 | `StarDreamTech/starclip` Actions secret | 密钥缺失时 |
| --- | --- | --- | --- | --- |
| 个人 Tap | `brew install --cask cpython666/tap/starclip` | `cpython666/homebrew-tap` → Deploy keys，允许写入 | `HOMEBREW_TAP_DEPLOY_KEY` | `update-tap` 失败并保留原 Cask |
| 组织 Tap | `brew install --cask stardreamtech/tap/starclip` | `StarDreamTech/homebrew-tap` → Deploy keys，允许写入 | `HOMEBREW_ORG_TAP_DEPLOY_KEY` | `update-org-tap` 写入 notice 与 step summary 后成功结束，不推送；`update-tap` 继续，不因此失败 |

- 两把私钥都只放在 `StarDreamTech/starclip` 的 Actions repository secret 中。不使用本机 GitHub 登录令牌，也不复用应用更新签名私钥；私钥不得提交到 Git、写进日志或 Release。
- 组织 secret 未创建，或值只有空白时，视为未配置。密钥已经配置但检出、校验或推送失败时，只有 `update-org-tap` 失败；已经推送到个人 Tap 的提交不会回滚。
- 停用或轮换时同时处理对应 Tap 的 deploy key 与 Actions secret。默认 PR 验证不会执行写入 job。

`actions/checkout` 使用严格 SSH 主机校验并在 job 结束时清理凭据。其他工作流若拿到同一 secret，也拥有该 Tap 的写权限，因此仍应保护主仓库的工作流修改权限。

## 已完成的线上验证

2026-11-11，[0.2.5 正式 Release](https://github.com/StarDreamTech/starclip/releases/tag/v0.2.5) 正式发布后完成 [同步 42861273222](https://github.com/StarDreamTech/starclip/actions/runs/42861273222)，`verify` / `update-tap` 两个 job 均通过。当前远端 `Casks/starclip.rb` 为 **0.2.5**，DMG 为 8,406,871 字节，SHA-256 为 `7094d2c0b34cd1f8943e9fc42ad049198cb42ac82828ca81a3efe93eb4d775b2`；与匿名公开下载、Release digest、签名草稿一致。[机器人提交](https://github.com/cpython666/homebrew-tap/commit/52a9d65d938742c2188f15f9f12fc2ec0a842005) 与远端 main Cask 已回读核对。未安装或启动本机正式应用；原生升级证据见 [0.2.5 验证记录](verification.md#025-正式发布与原生升级验收)。

2026-10-27，[0.2.2 同步](https://github.com/StarDreamTech/starclip/actions/runs/40466604899) 的验证和写入 job 全部通过。Tap 的 [机器人提交](https://github.com/cpython666/homebrew-tap/commit/5e3ed7db8d459a6f3961b0461816406516add923) 将版本更新为 0.2.2；远端 Cask 的 SHA-256 与公开 DMG 的 `600784a9fc48ffab2702cdbef66628dc71a34061c7566242ffd9bf075a428cb4` 一致。

2026-10-29，[0.2.3 正式 Release](https://github.com/StarDreamTech/starclip/releases/tag/v0.2.3) 发布后自动触发的 [同步 run 40896366521](https://github.com/StarDreamTech/starclip/actions/runs/40896366521) 已完成，`verify` 与 `update-tap` 两个 job 均通过。公开 DMG 的实际字节、内部应用身份 / 版本 / arm64、严格签名，以及 Homebrew 格式、在线审计和下载检查均通过；写入前再次核对公开 latest 与附件身份。

2026-11-03，[0.2.4 正式 Release](https://github.com/StarDreamTech/starclip/releases/tag/v0.2.4) 发布后自动触发 [同步 41580841113](https://github.com/StarDreamTech/starclip/actions/runs/41580841113)，verify / update-tap 两个 job 全部通过。该轮远端 `Casks/starclip.rb` 为 **0.2.4**，DMG SHA-256 为 `640d2bd02a216ccd15a742cf6c47868390ad874440efede9c7c2d470f4df0ad2`，与匿名公开下载、Release digest 及已验收签名包一致。[机器人提交 `2c961507eea106de52b9e667ba029cc39826ab0f`](https://github.com/cpython666/homebrew-tap/commit/2c961507eea106de52b9e667ba029cc39826ab0f) 只改该 Cask，提交内容和远端 main blob 已回读核对。没有安装或启动本机应用；真实公开 0.2.3 到签名 0.2.4 的两平台自动重启与数据保留另有验收，见 [验证记录](verification.md#024-正式发布与原生升级验收)。

上述线上记录只覆盖当时的个人 Tap。2026-11-11 用 GitHub API 只读核对两个 Tap 的 `Casks/starclip.rb`，git blob 同为 `9f51bdec0235e99eb5fae0a1f4e4988ff2d69bc7`，版本 0.2.5，`url` / `homepage` 已指向 `https://github.com/StarDreamTech/starclip`。这不是 `update-org-tap` 的推送验收；组织 secret 配置后的正式 Release 实跑仍待完成。

Homebrew 审计使用步骤范围的 `HOMEBREW_GITHUB_API_TOKEN`，值来自已有只读 `github.token`，避免公共 runner 共用 IP 的匿名 API 限流。没有新建个人令牌、跳过在线审计或修改已公开安装包。

## 重试与预演

Actions → **Update Homebrew after release** → **Run workflow**：选择 `main`，输入已经公开的 tag。

- `dry_run = true`（默认）：执行完整验证并保留 `verified-homebrew-cask` artifact，两个 Tap 都不推送。
- `dry_run = false`：验证通过后实际更新两个 Tap。个人 Tap 缺少 `HOMEBREW_TAP_DEPLOY_KEY`、校验失败、网络失败或推送冲突会报错，保留该 Tap 原 Cask。组织 Tap 未配置 `HOMEBREW_ORG_TAP_DEPLOY_KEY` 时按上表跳过。组织密钥已配置但推送失败时，只让 `update-org-tap` 失败。修复后重跑相同 tag 即可。

如果未来使用 `GITHUB_TOKEN` 在另一个 workflow 内自动发布 Release，GitHub 不会因此触发后续 `release` workflow；必须显式使用 `workflow_dispatch` 调用本工作流，传入同一 tag 和 `dry_run=false`。人工发布 Release 可直接触发，不需要这一步。

参考：[GitHub Release 事件](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#release)、[GITHUB_TOKEN 的范围与事件限制](https://docs.github.com/en/actions/concepts/security/github_token)、[单仓库 deploy key](https://docs.github.com/en/authentication/connecting-to-github-with-ssh/managing-deploy-keys)、[Homebrew Cask Cookbook](https://docs.brew.sh/Cask-Cookbook)。
