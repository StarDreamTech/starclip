---
name: starclip
description: 星笺 Starclip 项目共享知识库。处理本仓库的功能开发、修复、设计、测试、文档或知识库维护前阅读，按主题查阅已确认决策、实现入口与验证边界；用户说“更新skill”时回顾当前整个对话并维护知识库。
---

# 星笺 Starclip

独立、本地优先的桌面剪贴板与代码收藏工作台，技术栈为 Rust / Tauri 2 / SQLite / Vue 3 / TypeScript / Vite。核心流程是复制、搜索、星标或分组，再进入星绘坊制作本地分享图片；无需账户、剪贴内容本地保存；0.2.3 提供可随时关闭、未配置时不采集的可选功能统计；当前未发布源码采用默认临时编号的三档统计模式，见 [接入说明](../../../docs/telemetry.md)。

## 关键约定

- 按根目录 [AGENTS.md](../../../AGENTS.md) 工作与维护知识库。先检查工作区、保护已有修改；文档更新不授予提交、推送或安装权限。
- 中文、星空主题、清晰用途说明。使用星码集、星绘坊、星码卡、月光笺、流星栏、星尘整理、星伴这些产品名称；第三方名称只在参考出处出现。
- 不用真实剪贴板、正式历史或个人数据做测试与截图，不将数据库、备份、日志或凭据入 Git。新安装为空且暂停记录，升级保留用户偏好。
- 区分源码实现、自动检查、浏览器演示、平台构建和原生实测。历史验收只证明对应版本与场景；当前范围以代码及 [README](../../../README.md)、[产品范围](../../../docs/product.md) 核对。
- 用户说明与当前实现不一致时分别记录，不能把代码里的现有值倒写成用户要求。原九款手写字体由用户本人创作；2026-10-29 已确认界面作者字段统一显示「来源于互联网」，具体展示、许可和验证边界见产品决策。

## 当前交接（截至 2026-11-02）

- 0.2.3 已于 2026-10-29 正式发布为 latest，Homebrew 已同步；签名 tag 固定为 `45ffe25`。2026-10-31 只读核对正式安装 `Info.plist` 的版本文字已为 0.2.3，尚未核对其二进制来源，本轮未替换；不再沿用发布当时本机 0.2.2 的现状描述。真实旧版升级与产物证据见 [开发验证](references/development.md)；macOS 0.2.2 这次升级需手动重开，新版修复在后续升级生效。
- 用户已明确授权并完成本轮 0.2.3 发布与 Homebrew 同步，该授权不扩张为本机安装或后续发版。当前仅采用免费 GA4 + Cloudflare Workers，Azure 开通已取消。2026-10-30 用户要求独立项目暂不处理，名称仍待商榷；「星随」仅作讨论称呼，未授权抽离。
- 最新源码与原生证据入口为 [开发验证](references/development.md)；动画星伴仍未达到用户期望的几十 MiB。更新 skill 只维护知识，不顺带安装、发版或操作云服务。
- 2026-11-02 [PR #14](https://github.com/cpython666/starclip/pull/14) 已合入 main（`509ec71`）：移植字体下载进度（`handwriting-font-progress`）与 `npm run build:preview:mac`，打包最后一行输出可复制的 `open` 命令，同时保留 #12 的 `npm run app`。#1–#14 的合并提交均在 main 历史中；旧实现存档 `wip/local-20261101` 与历史验收分支保留，不回灌重复实现。预览采用 Starclip Preview / `com.starclip.preview`、独立数据目录、无线上统计与更新源，不触发 Release / Brew；验证与限制见 [开发验证](references/development.md) 和 [合并记录](../../../docs/verification.md#本地工作区并入-main2026-11-01-至-11-02)。
- 同日跨分支审计确认：PR 已合并不等于旧本地数据与统计语义兼容。main schema 2 与本地 schema 3 在文件去重 / 明细、AI 边界、旧 JSON v4 和纸张恢复上存在缺口；统计的同名 v3 协议也有两种不兼容形状。生产 Worker 只读核实仍为正式 v1/v2，会拒绝 main 新 v3。随后已实现统一 schema 4 / JSON 4、文件明细保留与 AI 排除、papers 恢复，独立 macOS 原创旧 3 升级及重开核对通过。统计偏好保留显式三档、旧布尔 true 保守转 session，支持旧身份 / 缓存隔离与 HTTP 4；生产 Worker 已更新为兼容 v1/v2、两种历史 v3 及新 v4，GA 维度已创建，实际入库与发布状态分别核对，客户端尚未正式发布。实现与证据见 [跨分支审计](../../../docs/verification.md#数据结构与统计跨分支审计2026-11-02) 和 [统计部署核对](../../../docs/telemetry.md#跨分支兼容与生产部署核对2026-11-02)。

## 按需阅读

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
| 内存过大、隐藏窗口、历史规模与分页 | [架构与实现入口](references/architecture.md)、[开发验证与测量方法](references/development.md)、[数据与隐私边界](references/data-safety.md) |
| 本地剪贴活动统计（星迹统计）、计数口径与清空 | [数据与隐私边界](references/data-safety.md)、[星迹统计](../../../docs/clip-insights.md) |
| 静默埋点、离线补报、设备字段、GA4 / Worker | [数据与隐私边界](references/data-safety.md)、[开发验证](references/development.md)、[接入说明](../../../docs/telemetry.md) |
| 内置手写渲染、字体预览 / 按需下载及署名 | [产品与交互约定](references/product-decisions.md)、[架构与实现入口](references/architecture.md) |
| 实施升级、恢复与签名发布 | [数据与隐私边界](references/data-safety.md)、[开发验证](references/development.md)、[发布指南](../../../docs/releases.md) |
| GitHub Release 发布后自动同步 Homebrew | [Homebrew 自动化](../../../docs/homebrew-release.md)、[开发验证](references/development.md) |
| 选检查命令、做桌面验收、理解踩坑与待办 | [开发验证与已知限制](references/development.md) |

完整资料：[产品范围](../../../docs/product.md)、[星尘整理](../../../docs/cleanup.md)、[星伴](../../../docs/starpet.md)、[星芽设计](../../../docs/starpet-design.md)、[验证记录](../../../docs/verification.md)、[参考体验对照](../../../docs/reference-comparison.md)。
