---
name: starclip
description: 星笺 Starclip 项目共享知识库。处理本仓库的功能开发、修复、设计、测试、文档或知识库维护前阅读，按主题查阅已确认决策、实现入口与验证边界；用户说“更新skill”时回顾当前整个对话并维护知识库。
---

# 星笺 Starclip

独立、本地优先的桌面剪贴板与代码收藏工作台，技术栈为 Rust / Tauri 2 / SQLite / Vue 3 / TypeScript / Vite。核心流程是复制、搜索、星标或分组，再进入星绘坊制作本地分享图片；无需账户、剪贴内容本地保存；0.2.4 提供可随时关闭、未配置时不采集的三档可选功能统计，默认每次运行临时编号，见 [接入说明](../../../docs/telemetry.md)。

## 关键约定

- 按根目录 [AGENTS.md](../../../AGENTS.md) 工作与维护知识库。先检查工作区、保护已有修改；文档更新不授予提交、推送或安装权限。
- 中文、星空主题、清晰用途说明。使用星码集、星绘坊、星码卡、月光笺、流星栏、星尘整理、星伴这些产品名称；第三方名称只在参考出处出现。
- 不用真实剪贴板、正式历史或个人数据做测试与截图，不将数据库、备份、日志或凭据入 Git。新安装为空且暂停记录，升级保留用户偏好。
- 数据结构升级必须兼顾逐级与跳级，不假设用户安装过中间版本；完整快照与原库事务迁移、兼容矩阵见 [数据迁移](../../../docs/data-migrations.md)。
- 用户 2026-11-03 调整验证节奏：日常迭代优先定向回归与必要构建，避免每轮重复跑全量 / 双平台安装；集成、迁移和发版前再集中完整检查，见 [开发验证](references/development.md#环境与检查)。
- 区分源码实现、自动检查、浏览器演示、平台构建和原生实测。历史验收只证明对应版本与场景；当前范围以代码及 [README](../../../README.md)、[产品范围](../../../docs/product.md) 核对。
- 仓库 [中文 README](../../../README.md) / [English README](../../../README.en.md)、中英宣传图 / 文案和软件语言切换统一维护；目前支持简体中文与 English，新增功能同步两份 README 和相关素材，具体入口见 [分享说明](../../../docs/product-share.md)。两份 README 开头链接[项目介绍页](https://stardreamtech.github.io/projects/starclip/)。
- 用户说明与当前实现不一致时分别记录，不能把代码里的现有值倒写成用户要求。原九款手写字体由用户本人创作；2026-10-29 已确认界面作者字段统一显示「来源于互联网」，具体展示、许可和验证边界见产品决策。

## 当前交接（截至 2026-11-11）

- 2026-11-11 仓库已转到 `StarDreamTech/starclip`，路径不变。更新端点、应用内仓库 / Issue、分享文案与海报页脚已换 owner；`docs/verification.md` 历史链接，以及字体来源、个人 Homebrew Tap 和统计域名等非本仓地址的保留范围见 [开发验证](references/development.md#仓库地址2026-11-11)。
- 2026-11-12 `v0.2.6` 已公开为 latest。tag 指向 `fab37f19`，签名运行 [43050585288](https://github.com/StarDreamTech/starclip/actions/runs/43050585288) 的 head 是 `386a9856`，比 tag 只多 CI 与说明。更新验收改为核对 compare 路径和已发布资产 sha256，规则见 [开发验证](references/development.md)。0.2.5 → 0.2.6 原生升级尚未重跑。`tauri.conf.json` 的 updater endpoint、`updater.rs` 下载白名单和组织仓库 `latest.json` 的安装包 URL 都是 `StarDreamTech/starclip`。`cpython666/starclip` 是独立兼容仓库，当前冻结在 0.2.5，旧地址不跳转到组织仓库。已安装的 0.2.5 只接受该兼容下载前缀；发版后由维护者镜像同一份包，并只改写兼容仓库那份清单的 URL。签名不覆盖 URL。验收夹具响应兼容 `latest.json` 时做同样的运行时改写。`macos-updater-acceptance` / `windows-updater-acceptance` 默认基线改为 `public-0.2.5`。
- 2026-11-11 Homebrew 发布在同一份已审计 Cask 上写入两个 Tap：`cpython666/homebrew-tap` 使用 `HOMEBREW_TAP_DEPLOY_KEY`，缺失则 `update-tap` 失败；`StarDreamTech/homebrew-tap` 使用 `HOMEBREW_ORG_TAP_DEPLOY_KEY`，未配置时提示并成功跳过，不使个人 Tap job 失败。Cask 的 url 指向组织仓库 Releases；0.2.6 起 `renderCask` 的 homepage 改为项目介绍页。已发布的 0.2.5 Cask 仍是 GitHub 仓库页，要等下次正式 Release 同步。组织 Tap 自动推送尚未经过正式 Release 实跑，见 [Homebrew 自动化](../../../docs/homebrew-release.md)。
- 用户 2026-11-10 明确授权发布刚合并的功能，0.2.5 已于 2026-11-11 00:42:14（北京时间，UTC 2026-11-10 16:42:14） 公开为 latest，tag `948dfd4`；[签名构建](https://github.com/StarDreamTech/starclip/actions/runs/42848254909) 8/8、真实公开 0.2.4 → 0.2.5 两平台原生升级自动重启、结构 4 → 5 / 固定数据保留与旧自动快照清理通过，旧应用副本保留。8 个匿名附件及稳定 latest 与草稿字节一致，[Homebrew](https://github.com/StarDreamTech/starclip/actions/runs/42861273222) 2/2 及远端 Cask / DMG 摘要已核对。本机正式安装未替换，Markdown 两平台原生 PNG 导出、Windows 多图粘贴及最终拖放收件不扩大为已验收，详见 [发布记录](../../../docs/verification.md#025-正式发布与原生升级验收)。
- 历史 0.2.3 已于 2026-10-29 正式发布并曾设为 latest，Homebrew 已同步；签名 tag 固定为 `45ffe25`。2026-10-31 只读核对正式安装 `Info.plist` 的版本文字已为 0.2.3，尚未核对其二进制来源，本轮未替换；不再沿用发布当时本机 0.2.2 的现状描述。真实旧版升级与产物证据见 [开发验证](references/development.md)；macOS 0.2.2 这次升级需手动重开，新版修复在后续升级生效。
- 2026-10-29 用户已明确授权并完成 0.2.3 发布与 Homebrew 同步，那次授权不扩张为本机安装或后续发版；0.2.4 已于 2026-11-03 获得单独发布授权。当前仅采用免费 GA4 + Cloudflare Workers，Azure 开通已取消。2026-10-30 用户要求独立项目暂不处理，名称仍待商榷；「星随」仅作讨论称呼，未授权抽离。
- 最新源码与原生证据入口为 [开发验证](references/development.md)；动画星伴仍未达到用户期望的几十 MiB。更新 skill 只维护知识，不顺带安装、发版或操作云服务。
- 2026-11-02 [PR #14](https://github.com/StarDreamTech/starclip/pull/14) 已合入 main（`509ec71`）：移植字体下载进度（`handwriting-font-progress`）与 `npm run build:preview:mac`，打包最后一行输出可复制的 `open` 命令，同时保留 #12 的 `npm run app`。#1–#14 的合并提交均在 main 历史中；旧实现存档 `wip/local-20261101` 与历史验收分支保留，不回灌重复实现。预览采用 Starclip Preview / `com.starclip.preview`、独立数据目录、无线上统计与更新源，不触发 Release / Brew；验证与限制见 [开发验证](references/development.md) 和 [合并记录](../../../docs/verification.md#本地工作区并入-main2026-11-01-至-11-02)。
- 同日跨分支审计确认：PR 已合并不等于旧本地数据与统计语义兼容。main schema 2 与本地 schema 3 在文件去重 / 明细、AI 边界、旧 JSON v4 和纸张恢复上存在缺口；统计的同名 v3 协议也有两种不兼容形状。生产 Worker 只读核实仍为正式 v1/v2，会拒绝 main 新 v3。随后 [PR #15](https://github.com/StarDreamTech/starclip/pull/15) 已合入 main（`e987256`），统一 schema 4 / JSON 4、文件明细保留与 AI 排除、papers 恢复，独立 macOS 原创旧 3 升级及重开核对通过。统计偏好保留显式三档、旧布尔 true 保守转 session，支持旧身份 / 缓存隔离与 HTTP 4；生产 Worker 已更新为兼容 v1/v2、两种历史 v3 及新 v4，GA 维度及 Rust 两模式、新日活 / 心跳已在实时明细核对；日期卡片未隔离历史 v3 样例，v4 首次日期独立归属、历史汇总与跨天留存仍待验证，该轮结束时客户端尚未发布，现已随 0.2.4 发布。最终业务 head `7b1e845` 的 push / PR 两组均 7/7 成功，包含两平台构建和 fresh / upgrade 安装；本地 main 已同步，未替换正式安装。实现与证据见 [跨分支审计](../../../docs/verification.md#数据结构与统计跨分支审计2026-11-02) 和 [统计部署核对](../../../docs/telemetry.md#跨分支兼容与生产部署核对2026-11-02)。

- 同日 [PR #16](https://github.com/StarDreamTech/starclip/pull/16) 已合入 main（`f7255ff`）：星伴图库手动点选立即保存并同步已开启浮窗，关闭不唤醒；巡游 / 分类仍仅预览。已核对 macOS 原生点选 / 重启、5 项组件回归及完整 7 项 PR CI，Windows 点选 GUI 未单独实测；见 [验证记录](../../../docs/verification.md#星伴表情点选同步2026-11-02)。

- 同日 [PR #17](https://github.com/StarDreamTech/starclip/pull/17) 已合入 main（`b61206b`），查询性能第一轮实现 Store 全局数量 / 筛选选项缓存，联合按本连接变更、外部提交与 DDL 失效；事务 / 活动游标不复用，记录与隐私 / AI 权限仍实时查询，结构保持 4。原创两版完整查询基准、正确性与剩余扫描边界见 [查询性能](../../../docs/query-performance.md)；精确业务 head `2bcef07` 的 push / PR 两组均 7/7 通过，包含双平台构建与 fresh / upgrade，未单独做 GUI / IPC 延迟测量；该轮未发布的客户端改动现已随 0.2.4 发布。

- 2026-11-03 [0.2.4](https://github.com/StarDreamTech/starclip/releases/tag/v0.2.4) 已按用户本轮授权公开为 latest，tag `6d83640`，[签名流水线](https://github.com/StarDreamTech/starclip/actions/runs/41579229975) 8/8、真实公开 0.2.3 → 0.2.4 双平台自动重启 / 数据与快照保留均通过。本地独立旧 3 → 4 / 123 条、查询与标签失效、表情同步重开、Python 识别及两类 PNG 导出已通过。DMG 8.18 MB，分享改为约 8 MB；正式包未安装到本机。8 个匿名公开附件和稳定 latest 已核对与草稿完全相同，[Homebrew 41580841113](https://github.com/StarDreamTech/starclip/actions/runs/41580841113) 2/2 及远端 0.2.4 Cask / DMG SHA 已核对，完整证据及限制见 [发布与升级验收](../../../docs/verification.md#024-正式发布与原生升级验收)。

## 按需阅读

- 0.2.5 已发布 Markdown 图片与主界面三个并列创作入口；三款 MIT 主题、限制和 Chromium 导出 / 原生待验收边界见 [Markdown 图片](../../../docs/markdown-images.md)。

- 0.2.5 已发布图片窗口全屏 / 缩放 / 图集切换、数字分页 / 跳转、收藏四种排序 / 拖动 / 独立置顶；SQLite / JSON 5 可选收藏元数据与验证边界见 [产品范围](../../../docs/product.md#图片查看分页与收藏排序开发分支)。图文组合只保存一段配文 + 有序图片，不保留任意交错排版。
- 0.2.5 已发布自动快照保留策略：升级成功并校验当前库后清理旧版本自动点，同版本保留最近 3 份 + 7 天，恢复点仅内部使用，撤下列表 / 手动创建 / 还原入口与权限，历史手动点保留，见 [保留策略](../../../docs/recovery-retention.md)。
- 0.2.5 已发布严格图片去重、图文组合 / 多图图集与分享素材收藏；SQLite / JSON 5、埋点 v4 不变。入口、格式限制与测试见 [图文组合](../../../docs/image-collections.md)。
- 0.2.5 已发布 Issue #18 图片批量传出：多选、独立 PNG 文件列表复制、目录导出和原生拖出入口；临时副本 / 隐私 / 限制及原生验收边界见 [数据与隐私边界](references/data-safety.md#图片批量传出2026-11-03开发分支未发布)。尚不代表两平台拖放 GUI 全部验收。

以下源码路径均相对仓库根目录；主题资料只在涉及该任务时加载。

| 任务 | 资料 |
| --- | --- |
| 找代码入口、改前后端协议或窗口权限 | [架构与实现入口](references/architecture.md) |
| 改界面、偏好、卡片或星伴，保留已确认体验 | [产品与交互约定](references/product-decisions.md) |
| 改界面文案、语言切换或新增翻译 | [架构与实现入口](references/architecture.md)、[产品与交互约定](references/product-decisions.md) |
| 修改产品分享入口、文案或宣传图 | [分享素材说明](../../../docs/product-share.md)、[架构与实现入口](references/architecture.md) |
| 自定义主题色、渐变方向或本地 PNG 贴纸 | [自定义外观](../../../docs/appearance.md)、[产品与交互约定](references/product-decisions.md) |
| 数据开放、隐私规则、域名筛选或 AI 接入 | [隐私与来源规则](../../../docs/privacy-ai.md)、[AI 接入说明](../../../docs/ai-access.md)、[数据与隐私边界](references/data-safety.md)；[早期设计与待办](../../../docs/open-data-ai-plan.md) 不等于完整实现 |
| 规划星伴养成、检查贴纸来源与授权边界 | [养成规划与素材核对](references/growth-and-artwork.md) |
| 改采集、图片、保留策略、备份或迁移 | [数据与隐私边界](references/data-safety.md) |
| 内存过大、隐藏窗口、历史规模与分页 / 查询性能 | [架构与实现入口](references/architecture.md)、[开发验证与测量方法](references/development.md)、[数据与隐私边界](references/data-safety.md)、[查询性能](../../../docs/query-performance.md) |
| 本地剪贴活动统计（星迹统计）、计数口径与清空 | [数据与隐私边界](references/data-safety.md)、[星迹统计](../../../docs/clip-insights.md) |
| 静默埋点、离线补报、设备字段、GA4 / Worker | [数据与隐私边界](references/data-safety.md)、[开发验证](references/development.md)、[接入说明](../../../docs/telemetry.md) |
| 内置手写渲染、字体预览 / 按需下载及署名 | [产品与交互约定](references/product-decisions.md)、[架构与实现入口](references/architecture.md) |
| 实施升级、恢复与签名发布 | [数据与隐私边界](references/data-safety.md)、[开发验证](references/development.md)、[发布指南](../../../docs/releases.md) |
| GitHub Release 发布后自动同步 Homebrew | [Homebrew 自动化](../../../docs/homebrew-release.md)、[开发验证](references/development.md) |
| 选检查命令、做桌面验收、理解踩坑与待办 | [开发验证与已知限制](references/development.md) |

完整资料：[产品范围](../../../docs/product.md)、[星尘整理](../../../docs/cleanup.md)、[星伴](../../../docs/starpet.md)、[星芽设计](../../../docs/starpet-design.md)、[验证记录](../../../docs/verification.md)、[参考体验对照](../../../docs/reference-comparison.md)。

分支流程与本轮逐项合并记录见 [分支与发布工作流](../../../docs/development-branches.md)。2026-11-10 已将原工作区 104 个改动文件保存在临时快照，再将图片批量传出、自动备份清理、图集 / 收藏 / 历史浏览、Markdown 图片、实时预览及星伴把手通过 #28–#33 依次合入 main；最终业务 head `5f4e5da` 完整 CI 7/7 通过，main `e3bca33` 与其内容相同。旧分支可能已通过 squash 合并，不能仅凭 `--no-merged` 判断功能缺失或重复合并。合并状态以该文档和对应 PR 为准，合入主线不等于已发版或已完成全部原生 GUI 验收。
