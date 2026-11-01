# 可选使用统计：GA4 标准版与 Cloudflare Free

当前方案使用 GA4 标准版与 Cloudflare Workers Free，按用户要求只使用免费服务，不开通 Azure、不绑定银行卡、不升级付费计划。既有 v1 接收端已部署，固定事件与 macOS 隔离桌面事件已在 GA Realtime 核对。已发布的 0.2.3 新增静默离线缓存和 v2 设备字段；兼容 v1 / v2 的中继已部署，真实 Rust v2 固定批次返回 204，GA 三个设备维度已创建并核对。新设备参数已在 Realtime 核对；2026-10-29 已另行完成 GA4 非实时探索中的 `count` 汇总，以及 macOS 隔离应用进程级断网、重启恢复与恢复联网后的静默补报。长期持续观察和 Windows 原生断网仍未完成，具体证据与边界见下文；不能由 HTTP 成功或实时参数推定其他报表已验收。没有配置接收端地址的构建不收集、不发送统计；接收端缺少 Google 配置时拒绝转发。使用统计缺省开启、可随时切换；已保存的选择保留。2026-10-30 的本地代码把开关改为三档模式（不发送 / 每次运行随机编号 / 本机固定编号），新增运行心跳与 v3 协议，尚未部署、未核对报表、未发布，详见下文「未发布改动」一节。它与默认开启的「检查新版本」是两个独立开关。

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

上述验收进行时软件尚未发布；0.2.3 已于 2026-10-29 经用户明确授权[正式发布](https://github.com/cpython666/starclip/releases/tag/v0.2.3)。本机正式安装仍为 0.2.2，未替换。

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

## 未发布改动：按安装识别、运行活跃心跳与 v3 协议（2026-10-30）

本轮代码已完成，但**尚未部署 Worker、未在 GA 核对报表、未做原生实机验收、未发布**。以下描述的是本地代码与测试行为，不代表线上已生效。

### 三档统计模式

界面从「开 / 关」改为三档模式，三者互斥且都要用户显式选择：

| 模式 | 含义 | 编号来源 |
| --- | --- | --- |
| `off` | 不记录也不发送任何事件 | 无 |
| `session` | 默认档；每次运行换一个随机编号，无法跨启动关联 | 内存中的随机会话编号 |
| `installation` | 使用本机固定的随机编号，用于估算活跃安装数 | `usage-identity.json` 中的 128 位随机值 |

- 旧偏好迁移：已保存的 `enabled=false` → `off`，`enabled=true` → `session`；`installation` 永远不会由旧开关、默认值或升级隐式选中，只能由用户在设置里点选。
- 新安装沿用原有基础默认（`session`）。偏好文件损坏、字段多余或模式值未知时停止统计并保持关闭，不猜测用户的意图。
- 打开主开关只会回到 `session`；离开 `installation` 会删除本机身份文件，因此再次选择该模式会得到一个新的编号。

### 本机统计标识的存放

`installationId` 是 `getrandom` 生成的 128 位随机值，表示为 32 位小写十六进制，只存放在默认应用配置 / 数据目录的 `usage-identity.json`：

- 独立于剪贴库：不写入 SQLite、不进入历史 JSON 导出、完整恢复点与数据目录迁移。
- 文件带版本号，读取上限 512 字节，拒绝符号链接、超大文件与格式不符的内容。
- 原子写入（临时文件 + `create_new` + `fsync` + `rename`），Unix 上权限为 `0600`。
- 只在 `installation` 模式下读写；创建或持久化失败会停止该模式的发送，并在设置里显示本地错误，不静默降级为会话模式。
- 重启、应用升级、剪贴数据目录迁移后保持不变；设置里提供「重置统计标识」，会删除旧文件、清空待发计数并生成新编号，删除失败会明确提示。
- 完整编号不返回前端、不写日志、不进入任何错误文案。

### 运行活跃心跳 `app_running`

- 只表示「进程在运行并尝试上报」，不代表使用时长，也不代表有人在使用。
- 启动后上报一次，之后每小时一次；复用已有的后台调度合并进现有批次，不新增线程或轮询。
- 系统睡眠恢复后最多补一次，不写离线计数缓存、不回放旧心跳。
- 关闭统计、重置标识、切换模式会清空心跳状态与待发批次。
- 功能计数仍使用有界缓存（72 小时 TTL、每种最多 100 次、逐步退避、静默失败）；缓存格式版本升到 2 并校验计数数组长度，避免旧九项数组与新事件错位。

### v3 协议

| 版本 | 字段 | 用途 |
| --- | --- | --- |
| v1 | `version` `sessionId` `appVersion` `platform` `events` | 已发布客户端 |
| v2 | v1 + `device` | 0.2.3 起 |
| v3 | v2 + 可选 `installationId` | 本轮；`installation` 模式才带 |

- Worker 同时接受 v1 / v2 / v3，各版本使用精确字段白名单，未知字段、非法编号、重复事件、超长或异常计数一律 400。
- v3 用 `installationId` 转换为 GA4 `client_id`（复用两段十六进制转十进制并以点连接）；`session` 模式仍用 `sessionId`。不发送 `user_id`，不注册自定义维度。
- 新增低基数事件参数 `identity_mode`（`session` / `installation`）。它需要注册为事件级自定义维度才能在报表里过滤，本轮未注册，只随请求发出。
- 设备字段白名单、广告用途 `DENIED`、超时不跟随重定向、IP 转发头固定占位均保持不变；不声称网络层匿名。
- Worker 仍不保存原始请求，Observability 保持关闭。

### 统计口径与 GA4 探索配置

以下口径用于自建探索，**本轮尚未在线上核对**，需先部署 Worker 再按步骤验收：

1. **功能使用次数**：自定义指标 `count` 求和。探索用「自由形式」，维度放 `eventName`（或「事件名称」），指标放 `Usage count`（`count` 的自定义指标）；不要用「事件数」，它只统计批次条目数。排除 `app_running`：加过滤条件 `eventName` 不等于 `app_running`。
2. **运行活跃安装数**：维度 `eventName` 过滤 `app_running`，指标用「活跃用户」。此时 GA4 的 `client_id` 就是上报方的编号，因此数值近似等于「本周期内上报过心跳的安装数」。
3. **周 / 月活跃安装数**：同上，日期范围改为最近 7 / 28 天，或按周、月做对比。
4. **启动次数**：`app_started` 的 `count` 求和；`client_id` 去重数即「有启动的安装数」。
5. **版本与平台分布**：维度 `app_version`、`platform`，指标用事件数或 `count` 求和；`app_arch` / `os_major` / `device_model` 只在 v2 / v3 存在。

**`identity_mode` 的现状**：本参数随 v3 一起发送，但要在 GA4 探索里按它过滤，必须先把它注册为事件级自定义维度。本轮遵守「不新增自定义维度」的约束，**没有注册**，因此当前只能在 Realtime 看到该参数键，不能用于报表过滤。注册前请先确认：`session` 与 `installation` 两种模式的 `client_id` 混在同一份数据里，按「活跃用户」算出的安装数会被高估（每次运行都算一个新用户），上面的第 2–4 条口径只对全部使用 `installation` 模式的数据成立。

限制与已知偏差：

- `session` 模式的 `client_id` 每次运行都变，跨启动不可关联；只有 `installation` 模式能算安装级指标。两种模式混在同一张报表会低估安装数、高估用户数。
- 用户可能在周期中途切换模式或重置标识，同一台机器会表现为多个安装。
- 心跳每小时一次且只在应用运行时产生：长期不打开应用的安装会掉出活跃口径；崩溃或强制退出会丢失当小时心跳。
- 事件按服务端接收时间入库，不还原发生时刻；补报可能重复或漏计。
- 没有 `user_id`、没有登录态，不能用于识别个人或计算真实留存；GA4 的会话与参与时长在本方案下不可靠。
- 广告个性化与广告用途固定为 `DENIED`，报表不得用于广告投放归因。

## 收集范围与界面

入口：**设置 → 规则与隐私 → 可选使用统计**。界面显示是否配置、当前模式（不发送 / 每次运行随机编号 / 本机固定编号）、固定事件与待发次数（含从缓存恢复的次数）。发送成功与网络失败均静默处理，不弹提示、不打断其他功能；保存模式、重置标识或清空本地缓存失败属于用户操作的本地保存问题，仍应明确反馈。没有旧偏好时默认开启（`session`）；已保存的选择保留，偏好损坏或读失败时停止统计。只有已配置接收端且模式不为 `off` 才计数，可随时切换；切到 `off` 会清空未发送计数与本地缓存，已经开始发送的请求无法撤回。

| 事件 | 计数含义 |
| --- | --- |
| `app_started` | 统计开启时启动应用 |
| `studio_opened` | 打开星绘坊 |
| `card_exported` | 成功导出卡片 |
| `backup_exported` | 成功导出备份 |
| `backup_imported` | 成功导入备份 |
| `update_check_completed` | 完成更新检查 |
| `update_download_completed` | 更新包下载完成并通过签名校验；失败、取消或重复点击已下载包不计数 |
| `update_install_started` | 确认版本有效且已进入安装阶段；是安装尝试，不表示成功 |
| `font_downloaded` | 成功下载字体 |
| `app_running` | 进程在运行并尝试上报（启动后与每小时一次的心跳）；不代表使用时长 |

v3 发送协议版本、应用版本、系统类别、上述事件及次数、基础设备字段，以及仅在 `installation` 模式下出现的 `installationId`：

| 字段 | 允许范围 |
| --- | --- |
| `device.architecture` | 当前应用二进制架构：`aarch64` / `x86_64` / `x86` / `arm`，否则 `unknown`；运行于兼容层时不冒充物理 CPU 架构 |
| `device.osMajor` | macOS 主版本；Windows 只区分 `10` / `11`；Linux 内核主版本；不可识别时 `unknown` |
| `device.model` | Mac 的限定格式公开产品代号或 `Mac`；Windows 的固定产品系列白名单或 `WindowsPC`；Linux 固定 `LinuxPC` |

客户端和 Worker 都限制字段和值，不把任意固件文字、细分配置当作机型上报。**不发送剪贴内容、图片、文件名、标题、搜索词、链接、路径、错误原文、账号、序列号、电脑名称、用户名或系统账户标识。** 会话编号只存在于内存，不写入磁盘；`installation` 模式的 128 位随机编号只写进独立的 `usage-identity.json`，不进入剪贴库、历史导出或恢复点。GA4 必需的 `client_id` 由这两种编号转换而来。设备信息只在发送时附带，不进入本地统计缓存。

固定计数暂存在默认应用数据目录的 `usage-counts.json`，不跟随剪贴历史数据目录迁移，也不进入历史 JSON 或完整恢复点。文件只包含缓存格式版本（当前 2）、应用版本、创建时间和 9 类功能计数；读取上限 2 KiB，每事件最多 100 次，最长保留 72 小时。同版本重启恢复未过期计数，应用版本变化则丢弃旧缓存；损坏、超限、未来时间、计数数组长度不符及不受支持的缓存不恢复。缓存不含设备字段、会话编号、安装编号或任何剪贴内容。

约每分钟、发送前后及正常退出时尝试写缓存；网络请求不阻塞功能操作。写盘失败静默保留内存数据以便后续再试，异常终止可能丢失最后一分钟的新增计数。网络失败后在本次运行按 1 / 5 / 15 / 60 分钟退避，之后最多每小时一次；退避状态不落盘，重启从正常调度重新开始。关闭统计清空计数与缓存，并更换临时会话身份；请求中途撤回不会让旧批次的失败回调重新恢复已清空计数。

一般事件约每分钟发送一次，更新下载完成和安装开始时会尽快在后台发送，不让统计请求阻塞更新；`app_running` 心跳按启动后与每小时一次并入同一批次。关闭、升级时舍弃缓存、异常退出或安装器终止进程可能使计数丢失；请求已被接收但回复丢失时，补报也可能重复。离线事件按服务端接收时间汇总，不还原事件发生时刻，不能以安装尝试数推断更新成功率。可按服务端收到的日期、应用版本与系统汇总下载 / 更新趋势；`session` 模式不含持续编号，`installation` 模式虽能区分安装，仍不是账号级用户追踪。此处只统计应用内更新，不代表 GitHub / Homebrew 安装包的全部下载量。

Worker 源码兼容 v1、v2 与 v3，只接受 `POST /collect` 的 JSON；上限 4 KiB、10 种事件（9 类功能事件 + `app_running`）、每种 1–100 次，拒绝未知字段、未知事件、重复事件与异常值；v2 / v3 还验证设备字段白名单，v3 的 `installationId` 必须是 32 位小写十六进制。默认每个有效批次只向 Google 发出一次请求；GA 配置无效时返回 503，完全不外发。`ENABLE_APPLICATION_INSIGHTS` 未设置或为 `false` 时，不校验微软连接字符串、不请求 Azure；即使残留连接字符串，也不会启用微软通道。只有显式设为 `true` 才校验并启用可选双发。供应商请求最长等待 5 秒，不跟随重定向；启用的通道任一失败均返回 503；Worker 本身不重试，客户端按上述退避机制重新提交。没有跨请求去重，供应商部分成功时后续补报也可能重复。

Worker 不保存请求，不打印密钥、载荷或错误原文；模板关闭 Worker Observability。不复制用户的 Cookie、User-Agent 或请求头，并为 IP 转发头设置固定占位值；参见 [Cloudflare Worker 子请求的 IP 规则](https://developers.cloudflare.com/fundamentals/reference/http-headers/#cf-connecting-ip-in-worker-subrequests)。相同发送逻辑的独立无 secret 探针已在回显端核对未观察到原始 IP / Cookie / User-Agent；这不是 Google 接收侧的完整抓包。接收端及其云平台仍会在网络层接触来源 IP，平台基础服务政策与日志设置需单独核查，不能把本实现理解为网络层匿名化。

## 1. 准备 Google Analytics 4 标准版

使用 [Google Analytics 免费标准版](https://marketingplatform.google.com/about/analytics/)，不选择 Analytics 360 或其他付费附加服务。

1. 在 [Google Analytics](https://analytics.google.com/) 创建星笺专用账号 / GA4 属性，设置报告时区，按 [官方创建步骤](https://support.google.com/analytics/answer/9304153) 添加 Web 数据流；网址填写星笺的公开项目介绍页，无须在桌面应用中加入网页采集脚本。
2. 在该数据流中取得 `G-…` 格式的 Measurement ID；进入 **Measurement Protocol API secrets** 创建 API secret。只放 Worker 服务端，不能写进客户端、Git、日志或截图。
3. 关闭不需要的数据共享、广告个性化和增强型衡量；本代码也固定发送广告用途与广告个性化 `DENIED`，不发送广告标识或账号信息。
4. 在「自定义定义」创建事件级自定义指标，事件参数填 **`count`**、单位为标准；可将 `app_version` 和 `platform` 注册为事件级自定义维度；当前 v2 的 `app_arch`、`os_major`、`device_model` 三个事件级维度已创建并在表格核对；三个参数及公开 Mac 产品代号也已在 Realtime 核对，但维度配置与实时参数不能代替常规汇总报表验收。看功能使用次数时汇总 `count`，GA4 默认的事件数量只代表聚合批次中的事件条目数。

协议格式依据 [GA4 Measurement Protocol 参考](https://developers.google.com/analytics/devguides/collection/protocol/ga4/reference?client_type=gtag)。此方案仅发送自定义事件，未接入 gtag / Firebase 自动采集；[Google 说明纯服务端方案仅有部分报表可用](https://developers.google.com/analytics/devguides/collection/protocol/ga4#full_server-to-server)。本实现不补造会话或停留时长，不适合据此计算准确活跃人数、留存或参与时长。

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

这里的尖括号需要替换；不要原样构建。接收端地址改变后需重新构建客户端。没有旧偏好时统计默认开启；最终用户仍可随时关闭，构建配置不会覆盖已经保存的关闭选择。

## 4. 验证与运行边界

本地纯模拟检查，无真实请求：

```sh
node --experimental-strip-types --test tests/telemetry-relay.test.ts
```

真实 Rust sender 测试默认忽略。已获授权的接收端验收可使用测试专用环境变量显式执行；只发送 9 种固定事件各 1 次，批次版本为 `0.0.0-telemetry-verification`，不访问 SQLite 或剪贴板：

```sh
STARCLIP_TELEMETRY_TEST_ENDPOINT='https://<worker>.<账户子域>.workers.dev/collect' \
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml \
  --no-default-features --lib \
  telemetry::tests::live_relay_accepts_original_fixed_counts \
  -- --ignored --exact --nocapture
```

其中尖括号需替换为已授权验收的真实地址；成功只断言接收端返回 204，不自动查询 GA 报表。关闭后的无批次检查在同一测试内完成；原生设置与重启行为需另行按本页记录的隔离应用场景验收。

部署验收使用隔离应用和原创固定事件，不能用真实剪贴内容。先用 [Google 的验证服务器](https://developers.google.com/analytics/devguides/collection/protocol/ga4/validating-events) 核对载荷；再确认测试版已配置接收端、默认开关开启，完成一次固定功能操作，检查待发计数、发送结果和 GA 报表，并确认关闭后无请求。Google `/mp/collect` 的 2xx 仅表示收到 HTTP 请求，**不保证事件有效或已入库**；不能只凭 HTTP 成功判定报表接通；用户界面不会显示发送成功或失败通知。

本轮已确认 Workers Free、关闭日志 / traces，并完成上述有限范围的出站头探针；后续部署继续核对这些配置并监测账号用量。公开匿名接收端可以被伪造请求，桌面包内嵌「共享密钥」无法解决这一点。本模板只限制单次请求大小与次数，没有持久限流、用户认证或去重库；防滥用措施应保持在免费方案内，不自动启用付费服务。

Cloudflare Workers Free 每个账号每天共 **100,000 次请求**，同账号多个 Worker 共享，UTC 00:00 重置。当前 `workers.dev` 接收端超限后请求被拒绝（Error 1027），不会自动升级为付费计划；不同产品分部署不增加账号免费额度。此额度按进入 Worker 的请求计算，不是事件 `count` 总和。参见 [官方限制](https://developers.cloudflare.com/workers/platform/limits/#daily-requests) 与 [官方定价](https://developers.cloudflare.com/workers/platform/pricing/)。

既有 v1 仅 Google 方案已有生产中继部署、真实 Rust 发送、GA Realtime 事件 / 参数、macOS 隔离原生开关 / 重启验收及 GitHub 构建变量核对。本轮兼容 v1 / v2 中继已部署，真实 Rust v2 固定批次返回 204，设备维度配置已核对，macOS 隔离分页 / 默认日常表情 / 静默发送场景已通过；新设备参数及公开 Mac 产品代号的 Realtime 接收已核对。这些能力已随 0.2.3 经用户授权发布。2026-10-29 已在 GA4 非实时探索中分别核对固定范围的事件条目数与自定义 `count` 使用次数，并完成 macOS 隔离应用进程级断网、重启恢复和恢复联网后的静默补报。该证据不覆盖系统 Wi-Fi 切换、全部网络异常、长期持续观察或 Windows 原生断网；出站头证据仍限于相同发送逻辑的回显探针。正式发布不扩大上述平台或场景的验收范围；本机正式安装仍为 0.2.2，未替换。

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
