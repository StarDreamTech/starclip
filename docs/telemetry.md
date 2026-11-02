# 可选使用统计：GA4 标准版与 Cloudflare Free

当前方案使用 GA4 标准版与 Cloudflare Workers Free，按用户要求只使用免费服务，不开通 Azure、不绑定银行卡、不升级付费计划。2026-11-02 源码已统一为三档统计模式与上报协议 4（客户端尚未正式发布），生产中继已兼容正式 v1/v2、两种历史 v3 与新 v4，当前状态见下文。既有 v1 接收端已部署，固定事件与 macOS 隔离桌面事件已在 GA Realtime 核对。已发布的 0.2.3 新增静默离线缓存和 v2 设备字段；兼容 v1 / v2 的中继已部署，真实 Rust v2 固定批次返回 204，GA 三个设备维度已创建并核对。新设备参数已在 Realtime 核对；2026-10-29 已另行完成 GA4 非实时探索中的 `count` 汇总，以及 macOS 隔离应用进程级断网、重启恢复与恢复联网后的静默补报。长期持续观察和 Windows 原生断网仍未完成，具体证据与边界见下文；不能由 HTTP 成功或实时参数推定其他报表已验收。没有配置接收端地址的构建不收集、不发送统计；接收端缺少 Google 配置时拒绝转发。新安装默认使用每次运行随机编号，可随时关闭；已保存的关闭选择保留。它与默认开启的「检查新版本」是两个独立开关。

本方案不需要自行购买服务器或域名：桌面端通过 HTTPS 向 Cloudflare Worker 发送精简计数，Worker 默认只转给 Google Analytics 4（GA4）。接收端可使用 Cloudflare 提供的 [workers.dev 地址](https://developers.cloudflare.com/workers/configuration/routing/workers-dev/)。已有 Microsoft Application Insights 通道仅保留为显式启用的可选模板，不属于当前接入计划；不集成网页录屏、点击热图或自动采集 SDK。

## 既有 v1 接入与验收（2026-10-28）

- Google：星笺 GA4 属性、Web 数据流、API secret、事件级 `app_version` / `platform` 维度及标准单位的 `count` 指标均已确认。合成载荷通过 `/debug/mp/collect` 的结构验证后，另经正式接收端发送；初次 Realtime 已看到 `studio_opened` 与 `font_downloaded` 各 1 条事件，并核对 `studio_opened` 的 `count=7` 参数。Realtime 的 `studio_opened → app_version` 还分别显示原生版本 `0.2.2` 与合成版本 `0.2.2-telemetryverify.20261028` 各 1 条，确认原生桌面事件已到实时报告。最终 Realtime 共 14 条、全部 9 种固定事件：`studio_opened` 3 条，`app_started` / `font_downloaded` / `update_check_completed` 各 2 条，其余 5 种各 1 条；与原生 3 条、初始合成 2 条、Rust 固定 9 条相符。这是实时事件条目数，不是常规报表 `count` 汇总。
- Cloudflare：当时控制台已确认账户使用 Free，Worker 已改名 `desktop-usage-relay`；v1 代码经 API 部署，当时远端 `worker.mjs` 与本地 SHA-256 均为 `a081d50ba6221fd8736882a7423972d0bc2a16db3a03369de5aa5f898cb6f02b`，两个 GA secrets 已绑定。`ENABLE_APPLICATION_INSIGHTS=false`，控制台日志、跟踪与问题开关均关闭；当前接收端为 `https://desktop-usage-relay.cpython666.workers.dev/collect`。实际运行发现 `redirect: 'error'` 不受 Workers 支持，已改为 `manual` 并拒绝 3xx，不跟随重定向。
- macOS 原生：独立标识 `com.starclip.telemetryfreeverify20261028` 的 0.2.2 验证包全程暂停、0 条记录。统计默认开启且接收端正确；打开一次空白星绘坊后自动发送成功、无待发事件。关闭统计后再次打开星绘坊不计数、发送按钮不可用；完全退出并重启后关闭选择保留，仍暂停、0 条记录、尚未发送。测试应用已退出，未替换正式安装。
- 出站头：使用相同 `send` 逻辑的独立无 secret 探针，以 `https://httpbun.com/headers` 为回显目标，检查合成 Cookie / User-Agent 标记；探针返回 204，固定转发头、无合成 Cookie、无合成 User-Agent 标记、无原始客户端 IP 四项断言均通过。回显端响应的 `Server` 为 `cloudflare`；此证据仅说明回显端未观察到原始 IP / Cookie / User-Agent，不是 Google 接收侧的完整抓包，也不能证明网络层匿名。临时探针 Worker 已删除，仅保留正式接收端，正式接收端没有调试路由。
- 发布准备：GitHub 仓库变量 `STARCLIP_TELEMETRY_ENDPOINT` 已设置并回读为上述接收端地址，没有触发工作流或发布软件。截至该轮等待用户发布确认；当时常规报表自定义指标仍需传播后核对 `count` 汇总，Windows 未实机验证。该轮云端配置不代表软件已发布；0.2.3 后续经用户明确授权发布。Azure 开通已取消，不再作为待办。

仅 Google 方案已通过 `npm test` 156 项、Rust 无 desktop 特性核心测试 95 项（另有 3 项默认忽略）及 Web 构建。9 项中继回归覆盖默认只发 Google、禁用微软时忽略残留配置、显式启用时的严格校验、重定向拒绝及失败处理。另显式执行默认忽略的真实 Rust sender 测试 1 项通过，使用生产 HTTPS 发送函数发送 9 种固定事件、每种 `count=1`，不读取数据库或剪贴板。Apple Silicon 隔离 release app 为 14.00 MiB，strict/deep ad-hoc 验签通过；上述原生场景与 GA Realtime 属于当时实际验收，不能将这些证据扩大为 Windows 或常规报表汇总已验证；后续汇总验收另见下文。

## 0.2.3 新增能力与发布前验收（2026-10-29）

本轮实现静默发送、有界离线缓存和重试，增加 v2 设备基础信息；生产 Worker 继续接受旧 v1。以下是本轮新增证据，与上面的 v1 历史记录分别保留：

- Cloudflare：兼容 v1 / v2 的生产中继已部署，远端与本地 `worker.mjs` SHA-256 均为 `d127d1fb4b24f4cadff240ab0a4a29b90d371119358c0b3df304d106899b96e9`。日志、跟踪、问题开关仍为 `false`，`ENABLE_APPLICATION_INSIGHTS=false`，未启用微软通道。
- Rust sender：v2 通过真实 HTTPS 发送 9 种固定事件，每种 1 次，接收端返回 204。设备样例固定为通用 `Mac`、架构与系统主版本为 `unknown`，没有采集真实机型或剪贴内容；这证明中继接受 v2 请求，不证明 GA 参数已入库。
- Google：已新增并在自定义定义表格核对事件级维度 `app_arch`、`os_major`、`device_model`。Realtime 的 `app_started` 显示三个参数键，各 2 条；`device_model` 下钻显示公开 Mac 产品代号，2 条。证据位于本地忽略目录 `artifacts/ga4-v2-event-parameters.txt` 与 `artifacts/ga4-v2-device-family.txt`，具体硬件代号不写入 Git 文档。该阶段已验证新字段的实时接收，尚未验证常规 `count` 汇总；后续固定范围汇总另见下文，长期持续观察仍未完成。
- macOS 原生：独立应用 `com.starclip.usageverify20261029` 全程暂停，使用 123 条原创固定记录。主列表每页 50 条，第二页显示编号 73–24 的 50 条；全库搜索能找到仅在第三页的编号 1。星伴默认显示 26 个日常表情，额外心情开关未勾选；静默发送后无待发事件，也无成功提示。追加核对：在普通记录筛选中把原创编号 123 标记为隐私后，该记录移出当前页（普通记录共 122 条），已打开的详情立即折叠并显示手动保护依据，验证 `get_clips` IPC 可用。测试应用已退出，全程暂停。
- 自动检查：`npm test` 166 项通过，Rust 无 desktop 核心测试 106 项通过、另有 3 项默认忽略。该阶段离线缓存、异常与重试由单元测试覆盖，尚未进行原生断网验收；后续 macOS 进程级断网证据另见下文，Windows 本轮未实机验证。最终 Apple Silicon 独立 release 应用为 14.07 MiB，应用标识 `com.starclip.usageverify20261029`，strict/deep 签名核验通过。

上述验收进行时软件尚未发布；0.2.3 已于 2026-10-29 经用户明确授权[正式发布](https://github.com/cpython666/starclip/releases/tag/v0.2.3)。该次 2026-10-29 验收未替换本机正式安装；2026-10-31 只读核对 Info.plist 为 0.2.3，二进制来源尚未核对。

## 补充验收：常规计数汇总与 macOS 进程级断网（2026-10-29）

GA4 已保存私有探索「星笺功能使用次数（验收）」，日期范围为 2026-10-01 至 2026-10-28。实际表格包含 9 种固定事件，共 14 条事件，`Usage count`（事件参数 `count` 的自定义指标）合计 22 次。两列分别核对如下；这次验收补齐了固定范围的非实时计数汇总，不以此前 HTTP 204 或 Realtime 参数替代：

| 固定事件 | 事件数 | Usage count |
| --- | ---: | ---: |
| `studio_opened` | 3 | 9 |
| `font_downloaded` | 2 | 4 |
| `app_started` | 2 | 2 |
| `update_check_completed` | 2 | 2 |
| `backup_exported` | 1 | 1 |
| `backup_imported` | 1 | 1 |
| `card_exported` | 1 | 1 |
| `update_download_completed` | 1 | 1 |
| `update_install_started` | 1 | 1 |
| **合计** | **14** | **22** |

证据保存在忽略的 `artifacts/acceptance.noindex/ga4-standard-count-evidence.json`。Git 文档只记录上述固定测试汇总，不复制账号、属性编号或探索完整网址。该日期范围截至 2026-10-28，不把后面的 2026-10-29 断网测试计数混入这张表，也不代表长期报表持续稳定已验收。

macOS 使用独立 `com.starclip.acceptance20261029` 验收包与新空库，全程暂停剪贴采集。通过 `sandbox-exec` 只限制该应用进程的网络出站，没有切换系统 Wi-Fi 或改变全机网络设置：

- 首次离线运行记录固定 `app_started=1`、`studio_opened=1`，形成 86 字节缓存；字段符合既定格式 / 应用版本、创建时间与固定计数结构，无身份或设备信息。
- 正常退出后仍在出站限制下重启，分钟 checkpoint 后核对缓存恢复并累计为 `app_started=2`、`studio_opened=1`。
- 再次正常退出，解除该进程出站限制后重新启动，后台自动发送后缓存被删除，界面显示无待发事件，成功与失败均未出现 toast。

以上确认 macOS 进程级断网、正常退出 / 重启恢复及恢复联网后的静默补报；不是系统 Wi-Fi 切换或所有网络条件的验收，不据此宣称该次补报已逐条在 GA 报表核对。验收包构建于字体展示文案修改之前，统计代码未因此更改；这些证据不用于证明最新字体展示已原生验证。长期持续观察、Windows 原生断网仍未完成。该轮隔离验收未发布或替换正式应用；0.2.3 已在后续经用户授权发布。

## 当前收集范围与界面（协议 4，客户端尚未发布）

入口：**设置 → 规则与隐私 → 可选使用统计**。未配置接收端的构建不计数、不发送，也不改动统计侧文件；偏好读取失败时停止统计并保留身份与缓存，界面提示重新选择模式。统计与「检查新版本」独立保存。

| 模式 | 身份与统计范围 |
| --- | --- |
| `off` 不发送统计 | 不计数、不发送；清空待发批次、缓存与两个旧 / 新身份文件。删除失败可见且可重试，不能声称清理成功 |
| `session` 每次运行随机编号 | 新安装默认；只发送九种功能计数，每次启动换编号，不跨启动恢复旧会话缓存 |
| `installation` 本机固定编号 | 用户明确选择后保存随机编号；另有当日使用与在线心跳，可重置；不来自硬件或账号 |

旧显式 `{revision,mode}` 保留三档选择。旧 `{revision,enabled:false}` 转为 `off`，`true` 保守转为 `session`：已发布 0.2.3 与未发布主线使用同形布尔值，却分别代表临时 / 持续身份，无法据此推断持续授权。原主线布尔用户需明确选择固定编号；不凭残留身份文件猜测同意。偏好修订号保留。

| 事件 | 计数含义 |
| --- | --- |
| `app_started` | 统计开启时启动应用 |
| `studio_opened` | 打开星绘坊 |
| `card_exported` | 成功导出卡片 |
| `backup_exported` | 成功导出备份 |
| `backup_imported` | 成功导入备份 |
| `update_check_completed` | 完成更新检查 |
| `update_download_completed` | 更新包下载完成并通过签名校验；不表示安装成功 |
| `update_install_started` | 已进入安装阶段；是安装尝试，不表示成功 |
| `font_downloaded` | 成功下载字体 |
| `app_active` | 仅固定编号模式；每次运行每个 UTC 日最多一次，属于应用进程运行口径，不证明用户前台交互 |
| `app_running` | 仅固定编号模式；约每小时一次，单次 count=1；不落盘、不补报离线心跳，不代表使用时长 |

载荷包含版本、应用版本、系统类别、每次运行随机编号、`identityMode`、固定事件及次数和受限设备字段。`device.architecture` 为应用二进制架构白名单；`osMajor` 仅主版本（Windows 仅 10 / 11 / unknown）；`model` 为限定格式 Mac 产品代号、Windows 固定系列白名单或通用类别。**不发送剪贴内容、图片、文件名、标题、搜索词、链接、路径、错误原文、账号、序列号、电脑名称、用户名或硬件标识。**

固定编号模式另带 `installId`，仅已知创建时间时发送 UTC `firstSeenDay`；旧编号没有日期时不补造。`usage-install.json` 规范格式为版本 2、编号与可选创建时间，兼容主线旧文件和本地存档的 `usage-identity.json`。固定编号模式遇到身份冲突、损坏或读写 / 清理失败时停止按安装统计并提示；冲突文件保留，可显式重置，或修复文件后重试以保留原编号。临时模式清理残留安装编号失败时保留可重试提示，仍可用本次临时编号发送功能计数；关闭模式不发送。Unix 文件权限 0600，原子写入；重置、切模式、关闭会清理待发 / 在途计数并换 generation，旧异步回调不能把旧身份计数放回。

`usage-counts.json` 的缓存 3 使用事件名称、模式和身份 SHA-256 指纹，另含应用版本与创建时间；不含完整编号、设备字段或剪贴内容。上限 2 KiB、每事件最多 100 次、保留 72 小时；只有相同应用版本 / 模式 / 身份才恢复。旧 9 / 10 项数组缓存因身份归属不明而舍弃，不跨身份重报。固定编号可跨重启恢复；临时编号跨运行舍弃。两个侧文件均在默认应用目录，不跟随剪贴历史迁移，也不进入 JSON 或完整恢复点。

一般事件约每分钟发送；更新下载完成 / 安装开始尽快后台发送。网络失败按 1 / 5 / 15 / 60 分钟退避，之后最多每小时一次；退避不落盘。写盘失败保留内存数据待重试，异常退出可能丢失最后一分钟。已经发出的请求不能撤回，响应丢失或通道部分成功可能重复；离线事件按服务端接收时间汇总，不重建发生时刻。功能计数、应用内更新尝试、公开安装包下载量和真实安装成功率不能互相替代。

## Worker 协议与报表口径

SQLite schema、JSON 备份、缓存与 HTTP 协议独立编号。Worker 通过精确字段形状兼容以下请求；拒绝未知或混杂字段，v4 临时模式不允许日活 / 心跳：

| 协议 | 身份形状 | 新增事件 |
| --- | --- | --- |
| 正式 v1 / v2 | 每次运行 session；v2 含 device | 原九种 |
| 历史 main v3 | 必需 `installId` + `firstSeenDay` | `app_active` |
| 历史本地 v3 | 可选 `installationId`，无日期 | `app_running` |
| 新 v4 | 必需 `identityMode`；固定模式含 `installId`、可选日期 | 固定模式允许 `app_active` / `app_running` |

只接受 `POST /collect` JSON，上限 4 KiB，固定事件每种 1–100 次、不得重复。默认仅向 Google 发一次请求，配置无效 / 外发失败返回 503；微软只有 `ENABLE_APPLICATION_INSIGHTS=true` 才启用。5 秒超时、不跟随重定向、不跨请求去重、不保存或打印载荷 / 密钥。静态 IP 转发占位头保持不变；历史回显探针只证明回显端未观察到原始 IP / Cookie / User-Agent，不等于 Google 接收侧完整抓包或网络层匿名。

GA4 的 `client_id` 来自固定编号或运行编号，事件参数 `identity_mode` 用于区分口径，已知日期转 `user_properties.first_seen_day`。新 v4 **不虚构 `engagement_time_msec`**；历史 main v3 的固定 1 ms 映射只为旧协议兼容保留，不用于新客户端。按 `count` 自定义指标汇总功能次数；事件数量代表批次中的条目数。活跃安装数应筛选 `identity_mode=installation` 和 `app_active` 后查看去重用户；临时会话不能算跨启动独立用户，进程常驻不等于前台使用。只对已知 `first_seen_day` 做后续留存分组，未知日期不能当新安装。GA 自带活跃用户、参与时长、首次访问与留存概览可能不适用于纯 Measurement Protocol 自定义事件，应以实际探索口径验收。参见 [GA4 协议参考](https://developers.google.com/analytics/devguides/collection/protocol/ga4/reference) 和 [纯服务端方案限制](https://developers.google.com/analytics/devguides/collection/protocol/ga4#full_server-to-server)。

## 跨分支兼容与生产部署核对（2026-11-02）

实施前只读审计确认生产仅支持 v1/v2（模块 SHA-256 `d127d1fb4b24f4cadff240ab0a4a29b90d371119358c0b3df304d106899b96e9`）；两个历史 v3 互不兼容，均不能只按协议数字混用。审计与离线矩阵在忽略的 `artifacts/audit-compat-20261102.noindex/`。

按用户确认的顺序，先实现本地 schema 4 / JSON 4 / 偏好与身份兼容，再于 **2026-11-02 08:12:38 UTC** 更新生产 Worker。回读模块为 11,927 B、SHA-256 `2c5a2a2b9230a1977510f286ea1682d71e407fdc532a8164306c40d85abab827`，部署版本 `9ef49cba-c869-4e63-b6d2-092039a517f7` 占 100% 流量，源码与仓库一致。两个 GA secret 按类型保留，未读取其值；微软仍 false、Logpush false、无 tail consumer，上传显式关闭 observability / logs / traces / issues（settings 回读未返回 observability 字段，不能据此宣称另有 UI 核对）。回退源保存在忽略目录，不改服务名或客户端地址。

新增事件级 `identity_mode`、用户级 `first_seen_day` 已在 GA 自定义定义表格核对，原五个事件维度与标准 `count` 指标保留。真实 Rust sender 已经生产端点发送原创样例：临时模式九种、固定模式十一种，均获 204。GA Realtime 另行核对版本 `0.0.0-telemetry-verification` 的 `app_started=2`，`studio_opened` 的 `identity_mode` 为 session / installation 各一条，`app_active` / `app_running` 各一条，用户属性 First seen day 的 `2026-11-02` 为一位用户；兼容性 HTTP 样例使用另一测试版本，不混作 Rust 证据。摘要为忽略的 `artifacts/schema4-compat-20261102.noindex/ga4-live-acceptance.json`，其中列出逐项快照。新增协议的历史报表 `count` 汇总与跨天 cohort 留存仍待后续处理时间 / 日期，未验收；HTTP 204、离线契约和维度创建均不能单独代替实际入库。此次客户端尚未正式发布或同步 Homebrew；原生数据升级证据见 [验证记录](verification.md#结构-4-兼容修复与验收2026-11-02)。

## 1. 准备 Google Analytics 4 标准版

使用 [Google Analytics 免费标准版](https://marketingplatform.google.com/about/analytics/)，不选择 Analytics 360 或其他付费附加服务。

1. 在 [Google Analytics](https://analytics.google.com/) 创建星笺专用账号 / GA4 属性，设置报告时区，按 [官方创建步骤](https://support.google.com/analytics/answer/9304153) 添加 Web 数据流；网址填写星笺的公开项目介绍页，无须在桌面应用中加入网页采集脚本。
2. 在该数据流中取得 `G-…` 格式的 Measurement ID；进入 **Measurement Protocol API secrets** 创建 API secret。只放 Worker 服务端，不能写进客户端、Git、日志或截图。
3. 关闭不需要的数据共享、广告个性化和增强型衡量；本代码也固定发送广告用途与广告个性化 `DENIED`，不发送广告标识或账号信息。
4. 在「自定义定义」创建事件级自定义指标，事件参数填 **`count`**、单位为标准；可将 `app_version` 和 `platform` 注册为事件级自定义维度；当前 v2 的 `app_arch`、`os_major`、`device_model` 三个事件级维度已创建并在表格核对；三个参数及公开 Mac 产品代号也已在 Realtime 核对，但维度配置与实时参数不能代替常规汇总报表验收。看功能使用次数时汇总 `count`，GA4 默认的事件数量只代表聚合批次中的事件条目数。

协议格式依据 [GA4 Measurement Protocol 参考](https://developers.google.com/analytics/devguides/collection/protocol/ga4/reference?client_type=gtag)。此方案仅发送自定义事件，未接入 gtag / Firebase 自动采集；[Google 说明纯服务端方案仅有部分报表可用](https://developers.google.com/analytics/devguides/collection/protocol/ga4#full_server-to-server)。v1 / v2 不含持续编号，不能计算跨启动独立用户或留存；当前 v4 的口径见上文。本实现不测量停留时长，GA4 中的参与时长没有意义。

## 2. 配置接收端

可通过 Cloudflare 控制台或以下 CLI 步骤补齐配置；本轮已完成与待办范围以上面的接入状态为准。Wrangler 是部署工具，不会加入桌面包。继续操作前，先使本地部署名称与目标 Worker 一致，确保代码和 secrets 配置到同一个部署。

```sh
cd telemetry-relay
npx wrangler@4 login
npx wrangler@4 deploy
npx wrangler@4 secret put GA4_MEASUREMENT_ID
npx wrangler@4 secret put GA4_API_SECRET
```

每条 `secret put` 都通过交互提示粘贴对应值，不将值作为命令行参数。保持 `ENABLE_APPLICATION_INSIGHTS` 未设置或为 `false`；当前只需两个 GA 配置，无需微软连接字符串。第一次部署尚无 GA 配置，`/collect` 会返回 503。部署工具会显示自己的 `https://<worker>.<账户子域>.workers.dev`，客户端地址需加 `/collect`。项目内 `.dev.vars`、`.env` 和本地 Wrangler 状态已忽略，仍不要将真实密钥写入文档或测试。

Worker 主程序是 [`telemetry-relay/worker.mjs`](../telemetry-relay/worker.mjs)，配置在 [`wrangler.toml`](../telemetry-relay/wrangler.toml)。密钥管理见 [Cloudflare Secrets](https://developers.cloudflare.com/workers/configuration/secrets/)。`GA4_MEASUREMENT_ID` 不是密钥，也统一放服务端便于维护；真正的 GA4 API secret 不应出现在 GitHub Actions 的客户端构建参数中。

模板中的 `desktop-usage-relay` 只是可修改的部署名称，客户端使用构建时配置的完整 HTTPS `/collect` 地址。其他软件可复用模板，但应按产品独立部署、使用各自统计项目，并调整固定事件白名单与测试。当前没有产品标识或多产品路由，同一部署不能隔离多个产品的数据；部署名称或网址改变后，客户端需更新接收端地址并重新构建。

## 3. 让发布包知道接收端地址

本轮已设置并回读核对仓库变量，值为 `https://desktop-usage-relay.cpython666.workers.dev/collect`；没有因此触发工作流。以下为后续维护步骤。

1. 部署并验收成功后，在 GitHub 仓库 **Settings → Secrets and variables → Actions → Variables** 添加变量 **`STARCLIP_TELEMETRY_ENDPOINT`**，值为自己的完整 HTTPS `/collect` 地址。
2. 发布流水线会在构建时注入该变量；只包含接收端网址，不包含 Google secret 或任何供应商凭据。变量留空，发布包即保持未配置状态。
3. 本地构建可临时设置同名环境变量：

```sh
STARCLIP_TELEMETRY_ENDPOINT='https://<worker>.<账户子域>.workers.dev/collect' npm run tauri -- build
```

这里的尖括号需要替换；不要原样构建。接收端地址改变后需重新构建客户端。没有旧偏好时默认使用临时编号；最终用户可关闭或明确选择固定编号，构建配置不会覆盖已保存的模式。

## 4. 验证与运行边界

本地纯模拟检查，无真实请求：

```sh
node --experimental-strip-types --test tests/telemetry-relay.test.ts
```

真实 Rust sender 测试默认忽略。已获授权的接收端验收可使用测试专用环境变量显式执行；分别通过临时 / 固定编号发送原创功能计数，固定模式另验证当日使用和心跳；批次版本为 `0.0.0-telemetry-verification`，不访问 SQLite 或剪贴板：

```sh
STARCLIP_TELEMETRY_TEST_ENDPOINT='https://<worker>.<账户子域>.workers.dev/collect' \
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml \
  --no-default-features --lib \
  telemetry::tests::live_relay_accepts_original_fixed_counts \
  -- --ignored --exact --nocapture
```

其中尖括号需替换为已授权验收的真实地址；成功只断言接收端返回 204，不自动查询 GA 报表。关闭后的无批次由离线回归检查；原生设置与重启行为需另行按本页记录的隔离应用场景验收。

部署验收使用隔离应用和原创固定事件，不能用真实剪贴内容。先用 [Google 的验证服务器](https://developers.google.com/analytics/devguides/collection/protocol/ga4/validating-events) 核对载荷；再确认测试版已配置接收端、默认开关开启，完成一次固定功能操作，检查待发计数、发送结果和 GA 报表，并确认关闭后无请求。Google `/mp/collect` 的 2xx 仅表示收到 HTTP 请求，**不保证事件有效或已入库**；不能只凭 HTTP 成功判定报表接通；用户界面不会显示发送成功或失败通知。

本轮已确认 Workers Free、关闭日志 / traces，并完成上述有限范围的出站头探针；后续部署继续核对这些配置并监测账号用量。公开匿名接收端可以被伪造请求，桌面包内嵌「共享密钥」无法解决这一点。本模板只限制单次请求大小与次数，没有持久限流、用户认证或去重库；防滥用措施应保持在免费方案内，不自动启用付费服务。

Cloudflare Workers Free 每个账号每天共 **100,000 次请求**，同账号多个 Worker 共享，UTC 00:00 重置。当前 `workers.dev` 接收端超限后请求被拒绝（Error 1027），不会自动升级为付费计划；不同产品分部署不增加账号免费额度。此额度按进入 Worker 的请求计算，不是事件 `count` 总和。参见 [官方限制](https://developers.cloudflare.com/workers/platform/limits/#daily-requests) 与 [官方定价](https://developers.cloudflare.com/workers/platform/pricing/)。

既有 v1 仅 Google 方案已有生产中继部署、真实 Rust 发送、GA Realtime 事件 / 参数、macOS 隔离原生开关 / 重启验收及 GitHub 构建变量核对。本轮兼容 v1 / v2 中继已部署，真实 Rust v2 固定批次返回 204，设备维度配置已核对，macOS 隔离分页 / 默认日常表情 / 静默发送场景已通过；新设备参数及公开 Mac 产品代号的 Realtime 接收已核对。这些能力已随 0.2.3 经用户授权发布。2026-10-29 已在 GA4 非实时探索中分别核对固定范围的事件条目数与自定义 `count` 使用次数，并完成 macOS 隔离应用进程级断网、重启恢复和恢复联网后的静默补报。该证据不覆盖系统 Wi-Fi 切换、全部网络异常、长期持续观察或 Windows 原生断网；出站头证据仍限于相同发送逻辑的回显探针。正式发布不扩大上述平台或场景的验收范围；该次 2026-10-29 验收未替换本机正式安装；2026-10-31 只读核对 Info.plist 为 0.2.3，二进制来源尚未核对。

## 可选手动模板：Microsoft Application Insights（当前不启用）

本节仅保留已有模板的使用说明，当前不执行、不申请 Azure 订阅，也不要求用户绑卡。只有以后用户另行选择微软服务时才需要以下配置；其写入与保留可能产生费用，见 [Azure Monitor 定价](https://azure.microsoft.com/en-us/pricing/details/monitor/)。它不是 Clarity，不包含录屏、热图或自动采集 SDK。

1. 使用已有 Azure 订阅，按 [官方步骤](https://learn.microsoft.com/en-us/azure/azure-monitor/app/create-workspace-resource) 准备 Application Insights 资源并关联 Log Analytics workspace，另行核对预算、保留期限与每日上限。
2. 将资源的连接字符串通过 `npx wrangler@4 secret put APPLICATIONINSIGHTS_CONNECTION_STRING` 保存到目标 Worker；要求包含 `InstrumentationKey` 与显式 `IngestionEndpoint`，仅接受公有 Azure 官方 HTTPS 接收域名，不接受代理、非标准端口或主权云地址。
3. 确认配置后，在同一 Worker 显式设置 `ENABLE_APPLICATION_INSIGHTS=true` 才会校验并启用微软通道；未设置或为 `false` 时忽略残留连接字符串。启用后任一供应商配置无效均返回 503，完全不外发；每批最多两个请求，任一失败或微软部分接受（206）均返回 503；Worker 不重发，但桌面客户端会退避后重新提交，可能让已成功通道收到重复计数。
4. 若资源已禁用 local authentication，现有 ikey 发送方式不够，需要另行实施 Entra 身份鉴权，不关闭已有安全要求。`InstrumentationKey` 只是资源标识，不是授权令牌，见 [连接字符串与鉴权说明](https://learn.microsoft.com/en-us/azure/azure-monitor/app/connection-strings)。

微软事件不含运行随机编号；使用 [EventData 格式](https://github.com/microsoft/ApplicationInsights-JS/blob/master/shared/AppInsightsCommon/src/Telemetry/Event.ts)，次数位于 `customMeasurements.count`。206 处理依据 [官方发送器](https://github.com/microsoft/ApplicationInsights-JS/blob/master/channels/applicationinsights-channel-js/src/Sender.ts)。Application Insights 资源的 Logs 查询示例：

```kusto
customEvents
| where timestamp > ago(7d)
| summarize uses = sum(todouble(customMeasurements["count"])) by name
| order by uses desc
```
