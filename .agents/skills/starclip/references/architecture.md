# 架构与实现入口

以下路径相对仓库根目录。依据当前源码核对；这里只记录定位与耦合关系，不复制实现。

## 前端

| 路径 | 职责与修改注意点 |
| --- | --- |
| `src/main.ts`、`src/App.vue`、`src/starpet-shell.css` | 入口按 `?view=starpet` 动态加载桌面星伴或主应用；星伴使用独立外壳，不加载主应用全局样式。主应用管理列表、详情、设置草稿、弹窗和快捷键 |
| `src/i18n/`、`src/types.ts`、`src/api.ts` | 中文原文作键的简体中文 / English 目录、`t()` 插值与单复数、`errorText()` 原生错误映射；locale 即时切换并写入 `uiLanguage`；桌面端命令 `set_native_language` 仅主窗可用 |
| `src/types.ts`、`src/api.ts` | 前后端数据形状与调用入口；用 `__TAURI_INTERNALS__` 区分桌面 invoke 和原创内存演示。修改行为时检查两条分支及 Rust 类型 |
| `src/components/ProductShare.vue`、`src/product-share.ts`、`src/product-share-catalog.json` | 独立「分享」页，默认跟随界面 / 手选中英；2 类文案 × 2 语言、5 主题 PNG × 2 语言。按 `locale` 筛选，去掉 `-en` 后缀维持当前主题；预览 / 复制 / 保存使用同一语言，元数据与 Rust 共用 |
| `src/components/UpdateSettings.vue`、`src/components/UpdateNotice.vue`、`src/update-api.ts`、`src/update-state.ts` | 更新与恢复界面、独立自动检查开关、主界面空闲时的稳定版提示 / 单版本忽略、进度订阅、确认版本固定、恢复后重启保护；浏览器仅说明。偏好按 revision 合并，提示本身不发起网络检查 |
| `src/components/StartupSettings.vue`、`src/startup-api.ts` | 使用习惯中的三项独立即时保存开关：登录自启动、启动收起、图标显隐；读取 / 保存中保护释放，失败可重读。退出事件交由 `App.vue` 核对草稿与子面板状态；浏览器只说明桌面限定 |
| `src/privacy.ts`、`src/components/PrivacyRules.vue`、`src/components/AiSettings.vue` | 标签 / 来源 / 隐私筛选、规则与重扫、本地 AI 授权及连接配置；`src/api.ts` 包装命令，浏览器只作固定数据演示 |
| `src/components/DataSettings.vue`、`src/components/CleanupPanel.vue` | 数据位置、备份回执与目录操作；整理预览和确认。父设置的挂载状态由 `src/App.vue` 保留 |
| `src/components/CodeStudio.vue`、`src/code-card.ts`、`src/snippets.ts`、`src/code-language.ts` | 星绘坊在打开时按需加载；Canvas 代码卡片、语言与导出工具，卡片预览和 PNG 共用渲染 |
| `src/components/HandwritingPanel.vue`、`src/handwriting-preview.ts` | 字体预览图库、单款显式下载、已下载字体选择、本地预览调度、过期结果隔离与导出有效性；浏览器只展示预览，不下载字体或模拟渲染 |
| `src/components/HistoryPagination.vue`、`src/types.ts::HistoryQuery` / `HistorySummary` | 主列表与 AI 授权选择共用分页控件；各自维护页码 / 查询，不将当前页当作全库 |
| `src/components/ClipStatsPanel.vue`、`src/clip-stats.ts` | 「星迹统计」设置页：范围切换、日 / 周柱、类型、时段 / 星期、明细表与当前星系；按 `clips-changed` 防抖重读，清空期间保护窗口释放 |
| `src/components/ClipImage.vue` | 可见区加载图片，详情原图与缩略图分开取用 |
| `src/theme.ts`、`src/color.ts`、`src/styles.css` | 即时主题保存、扩散效果、颜色规范及全局样式 |
| `src/appearance.ts`、`src/appearance-api.ts`、`src/components/AppearanceSettings.vue` | 独立强调色 / 渐变 / 本地 PNG 外观、可读性调整、失败回退；`StickerScene.vue` 显示自定义图 |
| `src/components/UiSelect.vue`、`src/components/UiColor.vue` | 统一下拉和颜色控件；保留字符串、数字、null 值语义、键盘与合法 HEX 校验 |
| `src/components/StartupScreen.vue`、`src/components/OnboardingGuide.vue`、`src/components/StickerScene.vue` | 流星启动画面、四步教程和原创贴纸 |
| `src/components/StarPet.vue`、`src/components/StarPetSettings.vue`、`src/components/StarPetDesktop.vue` | 共享 SVG 角色、设置预览、原生浮窗 UI |
| `src/starpet.ts`、`src/starpet-api.ts`、`src/starpet-size.ts`、`src/starpet-motion.ts`、`src/starpet-behavior.ts`、`src/starpet-action.ts`、`src/starpet-locomotion.ts` | 表情定义、独立偏好通信、大小保存队列、运动、情境互动、主动动作姿态，以及拖动步态与贴边姿态 |
| `src/starpet-growth-catalog.json`、`src/starpet-growth.ts`、`src/starpet-growth-api.ts`、`src/components/StarPetGrowth.vue` | 前后端共用任务 / 动作 / 物品 / 成就目录；成长类型与存档校验、浏览器内存演示、专用命令 / 事件和成长设置页。预览不记奖，窗口按修订号接收状态 |
| `src/starpet-signal-api.ts`、`src/components/StarPetDesktop.vue` | Agent 临时状态 / 动作 / 短句；监听 `starpet-signal` 后读取 `get_starpet_signal` 快照，revision 防迟到覆盖，TTL与卸载清理，不走成长奖励命令 |

设置动作经 `interact_starpet` 校验并保存后，向已有星伴窗口发 `starpet-interaction`（action / animate / signalRevision）；`starpet-growth-api.ts::onStarPetInteraction` 负责订阅，`StarPetDesktop.vue` 只在该事件中播放，命令响应只更新成长，避免双播。触摸 / 拖动传 `animate=false` 保留本体现有反应。读取偏好、保存成长和清除旧 Agent 提示按 storage → store → ai 锁序；重复动作即使不增长经验仍可重播。

`starpet.ts::everydayStarPetMoods` 统一日常表情池：默认图库、桌面 `nextMood` 与设置巡游只使用 26 个正面 / 中性状态，排除 `puzzled/sad/crying/angry/nervous/error`。图库有显式展开入口，点选及保存仍接受全部 32 个稳定 ID；已保存的负面 mood 和 Agent 明确传入的 error 不被自动改写。`StarPet.vue` 的行为导演只调整姿态，动作结束回到外部指定 mood。

## Rust 与桌面

| 路径 | 职责与修改注意点 |
| --- | --- |
| `src-tauri/src/lib.rs`、`src-tauri/src/main.rs`、`src-tauri/Cargo.toml` | 默认启用 `desktop`；主程序在 Tauri 启动前分流 `ai` / `mcp` 模式。无默认特性可测存储与纯逻辑，不启动桌面 |
| `src-tauri/src/desktop/` | `mod.rs` 命令注册、共享状态、托盘、全局快捷键、原生窗口与重启；`clipboard.rs` 系统剪贴板读写与后台采集循环；`ai_broker.rs` 本机 AI 回环接口（阻塞 accept）；`tests.rs` |
| `src-tauri/src/startup.rs` | 系统登录项读取与显式启停，按应用标识隔离；核对可执行路径和可检测的系统禁用，macOS 后台运行审批仍是额外限制，不从数据库偏好重建注册 |
| `src-tauri/src/updater.rs` | 原生更新状态与互斥、正式下载源限制、旧应用备份；desktop/ 包装检查 / 下载 / 备份 / 安装命令 |
| `src-tauri/src/telemetry.rs`、`src-tauri/src/telemetry_device.rs` | 固定功能计数、临时会话、有界磁盘缓存与重试；架构 / 系统主版本 / 机型按白名单归一化，禁止原始固件内容或永久身份 |
| `src-tauri/src/clip_stats.rs` | 本机剪贴活动计数表、记录 / 清空、按本地时区汇总与现存历史构成；见 [数据与隐私边界](data-safety.md) |
| `src-tauri/src/capture.rs` | 开启时捕获当前内容、变更和重试状态机，以及敏感类型 / CF_HTML SourceURL / Windows 进程名等纯函数；本模块不访问真实剪贴板 |
| `src-tauri/src/clipboard_source.rs` | 仅 desktop：采集时读取剪贴板敏感标记、来源应用与浏览器来源页面（macOS NSPasteboard / NSWorkspace，Windows 剪贴板格式与所有者进程），失败视为无上下文 |
| `src-tauri/src/search_index.rs` | contentless FTS5 trigram 索引与触发器，≥3 字符关键词预筛候选后仍由 `starclip_contains` 确认；开启 FTS5 secure-delete；启动时缺表 / 缺触发器自动重建 |
| `src-tauri/src/store/` | `mod.rs` 类型与共享辅助；`schema.rs` 打开 / 升级；`preferences.rs`；`privacy_rules.rs` 规则 / 重扫；`history.rs` 分页查询；`clips.rs` 采集与写入（含 `clip_rich` HTML、文件记录）；`cleanup.rs` 删除 / 整理 / 保留；`backup.rs` JSON v3；`tests.rs`。所有内容写入在同事务扫描 |
| `src-tauri/src/privacy.rs`、`src-tauri/resources/bip39/` | 本地敏感检测、URL 解析与来源、关键词 / 精确域名规则、手动和自动标签区分；词表来源许可随资源保存 |
| `src-tauri/src/ai_access.rs`、`src-tauri/src/ai_transport.rs` | 会话连接、固定 ID + 随机身份授权、统一读取过滤、独立星伴权限与限时内存信号、审计及有界消息；stdio MCP / CLI 经本机桥接，不能直接打开数据库 |
| `src-tauri/tests/ai_stdio.rs` | 启动实际主程序子进程，验证 CLI 与 MCP stdin/stdout，不读取正式用户数据 |
| `src-tauri/src/starpet_growth.rs` | 成长、日限额、解锁、装备与存档校验；`store/` 独立事务存储，`desktop/` 在代码保存 / 导出成功后授奖并广播 |
| `src-tauri/src/share.rs` | 固定分享 ID、路径 / PNG 字节与尺寸校验；desktop/ 使用已打包资产并负责复制 / 保存，禁止任意内容或路径参数 |
| `src-tauri/src/images.rs` | RGBA/PNG 校验、像素摘要、缩略图和尺寸限制 |
| `src-tauri/src/appearance.rs` | 自定义外观白名单补丁、静态 PNG 结构与有界解码、512px 缩图；`store/` 独立偏好事务，四命令仅主窗授权 |
| `src-tauri/src/storage.rs` | `data-location.json` 定位、数据目录校验、SQLite 一致快照、迁移与采用已有目录；受管目录 `renderers` / `fonts` 共用占用计算、迁移与恢复摘要白名单，拒绝符号链接；旧 `handwriting_directory()` 仅用于外部渲染器配置兼容，内置字体必须位于 `current/fonts` |
| `src-tauri/src/handwriting.rs`、`public/handwriting-fonts/` | 未发布内置 CPU 渲染与受管字体：原手写仓库 9 项及 Google Fonts 14 项，固定 commit、限定字节数及 SHA-256，验证后启用，TTF 按需保存到 `current/fonts`；目录与 IPC 透传 `author` / `tags`，原 9 项按用户确认显示「来源于互联网」（英文 “Sourced from the internet”），14 个外部字体选项保留原作者；面板可筛手写体。安装包只有清单、静态 PNG 预览与分来源许可 / 提供说明，旧外部 CLI 配置仅保留迁移兼容 |

SQLite 主要表为 `clips`、`clip_images`、`clip_privacy`、`groups`、`preferences`。`clip_privacy` 的 JSON 元数据保存手动隐私、持续保护原因、规则扫描版本、手动 / 自动标签、域名来源和随机记录身份。偏好独立使用 `settings`、`onboarding_v1`、`starpet_v1`、`starpet_growth_v1`、`appearance_v1`、`privacy_rules_v1`、`update_preferences_v1`、`telemetry_preferences_v1`、`window_startup_v1`；教程、主题、外观、成长、规则、更新提醒与启动设置保存不能随意换成完整设置保存，后者带有清理副作用。`window_startup_v1` 仅保存启动收起与图标显隐，系统登录项不缓存进数据库。JSON v3 含记录、手动标签、隐私与规则，不包含其它偏好；成长另有独立存档。

普通设置中的 `Settings.pause_revision`（JSON `pauseRevision`）由 Store 管理，旧数据缺字段默认为 0；前端提交 `expectedPauseRevision`，`Store::save_settings_with_pause_guard` 在同一事务读当前版本并保存。期望版本过期时保留当前 `paused`，其他字段继续保存；只有暂停状态实际改变才递增版本，忽略 payload 伪造值。`App.vue` 刷新同步较新暂停状态与版本而保留其他草稿，`settingsDirty` 排除版本字段；这也覆盖 true → false → true 的 ABA 变化。入口为 `store/`、`desktop::save_settings`、`api.ts::saveSettings`。

## 历史加载与内存边界

存储仍为单个 SQLite（原图 / 缩略图 BLOB），不是分库。`CleanupSnapshot` 不再保留全量 `Clip` 正文；候选为 ID 与 32 字节 SHA-256，显示项仅最大的 100 条，事务删除前逐行重新读取校验原字段。`cancel_cleanup(token)` 仅匹配当前 token 才释放，主窗独占权限；`CleanupPanel` 在改筛选、卸载及迟到预览时取消。`privacy_rescan_plan` 用查询游标逐条读取正文，仍暂存待更新元数据。全局筛选集合和候选 ID 数组不是恒定内存。

历史分页由 `store::history_page(HistoryQuery)` 在 SQLite 执行搜索、分类 / 分组、标签、来源与隐私筛选，每页固定 50 条，按更新时间和 ID 稳定排序；`get_state` / `api.getState` 返回当前页正文及 `HistorySummary`。全库分类 / 分组数量、标签与来源选项单独聚合，不从当前页推算；浏览器演示返回同样形状。`App.vue` 换页替换列表，`AiSettings.vue` 独立用 `aiEligible` 分页搜索并保留跨页勾选（最多现有授权上限 500 条）；创建连接仍走后端权限校验。分页限制常驻列表正文，不声称全进程只保留 50 条对象或达到固定内存量。

分页相关复核不能只看当前页：详情和 AI 已选 ID 经主窗 `get_clips`（最多 500）重读，私密变更移出筛选时先遮住旧正文，删除后关闭详情，失败继续保护；迟到响应不能覆盖新选择。跨页 AI 勾选在刷新、翻页和预览前重新校验，移除已删除 / 私密 / 待扫描项。`has_pending_ai_history` 用 SQL EXISTS 检查全库状态，不为授权创建加载全库正文。

## 更新、隐私与本机接口

自动更新调度复用 `desktop::monitor`：启动约 20 秒后首次调度，每 60 秒检查资格；`store::record_update_check` 用持久化时间限制自动网络请求至少间隔 24 小时，失败、重启与手动检查都计入，时钟回退重置基准并等待完整间隔。开关 / 忽略版本独立保存，不走保留清理；检查按 storage → store → Updates 取得锁并在联网前放开，避免关闭开关后排队任务继续请求。`updater.rs` 排除预发布，自动检查保护 ready 包；只有 main 获得三项更新偏好命令。

图片的 `privacy.rs::Metadata::normalize_image_policy` 仅去掉旧版「图片尚未进行文字识别」原因、设扫描版本为 0，保留手动与真实原因；`clip_from_row` / `read_metadata` 读取归一化不写回，扫描 / 导入沿用同一方法。图片内部摘要不当作识别正文；可扫描标题，AI 资格仍明确排除 image。详情与 `ClipPrivacy.vue` 通过 `privacyReasonExplanation` 显示规则说明。

`desktop/` 中本机桥接只绑定回环临时端口，按存储冻结 → 记录 → AI 授权锁序读取并持锁至响应写出。CLI / MCP 在主应用未开启接入时不可读；`get_state` 是仅主窗可用的分页查询接口，不能用作 AI 数据出口。接口用法和边界见 [AI 接入](../../../../docs/ai-access.md)、[隐私与规则](../../../../docs/privacy-ai.md)。

## 窗口权限

- `src-tauri/tauri.conf.json` 配置主窗口 `main` 与 CSP；星伴窗口 `starpet` 由 Rust 创建。
- `desktop/` 收到主窗关闭先隐藏，再向 `App.vue` 发送带令牌的关闭请求；前端经 `api.hideWindow(release, requestId)` 确认安全空闲后销毁 WebView。0.2.3 修复将编号与 pending 状态同锁保存；主窗早订阅后调用仅 main 可用的 `get_main_window_close_request` 补读启动期事件。前端安全状态变化后续办，仅原生更新 / 存储暂缓时保留一个 1 秒重试计时器。编辑、星绘坊、教程、任务及尚未初始化时保护；设置页须父偏好无修改、全部已挂载子页 `canRelease === true`，隐藏草稿、凭证与确认态也保护。重新打开先作废旧令牌，在工作线程等待销毁完成并按配置重建，后台采集与托盘继续；只缓存本次运行的尺寸、位置和最大化状态，重新应用置顶，不持有 WebView。重建不重播启动画面，搜索 / 筛选 / 滚动等未持久化界面状态复位；Windows CI 已原生验证关闭后 HWND 销毁、二次启动重建及暂停状态；不扩大为全部草稿、多屏或快捷键场景。
- 星伴停用成功保存偏好后销毁窗口，重新开启时重建；失败保留既有偏好回滚处理，星伴位置不持久化。界面语言由 `set_native_language` 同步托盘菜单、主窗口与星伴窗口标题；托盘菜单项句柄保存在 `TrayItems`，语言切换时就地更新文字与提示。
- 0.2.3 启动设置：`tauri.conf.json` 关闭主窗自动创建，`desktop/` setup 根据 `WindowStartupPreferences.start_hidden` 决定是否创建；快捷键注册失败或窗口偏好损坏则显示主窗，星伴仍按原偏好恢复。托盘继续创建并按 `show_tray_icon` 设置可见，快捷键 / 单实例再次启动 / macOS `RunEvent::Reopen` 复用主窗恢复路径。`get_startup_preferences` 合并系统登录项与本地窗口偏好，`set_startup_preferences` 只接受三字段布尔补丁，外部副作用或持久化失败尝试回滚并报告错误；两命令和 `quit_starclip` 仅主窗授权，写入受存储 / 更新保护。`App.vue::quitFromSettings` 先检查普通草稿和子页 `canRelease`。本轮全量自动检查及 macOS 隔离原生验收通过；Windows 构建与登录项启停 / 隐藏启动 / 再次呼出的原生验收已通过，两平台真实注销重登仍待验证，边界见 `docs/verification.md`。
- 自定义命令同时涉及 `src-tauri/build.rs`、`src-tauri/permissions/windows.json`、`src-tauri/capabilities/default.json`、`src-tauri/capabilities/starpet.json` 和 `src-tauri/src/desktop/`。增加命令时一起核对注册与最小授权；`set_native_language` 只在 `main-commands`，星伴窗口不得调用。
- 隐私 / 标签 / 规则 / 重扫、AI 开关 / 授权 / 配置 / 审计命令仅主窗口可用；星伴不能借这些命令取得历史。AI 工具不允许改变权限或写入数据。
- 产品分享三个命令仅主窗口可用；读取固定打包素材，不授予星伴窗口剪贴板权限。剪贴写入与既有历史复制共用 helper，成功后才登记自采集抑制；保存使用专用命令，不走会授予成长经验的 `save_export`。
- 星伴命令以 `permissions/windows.json::starpet-commands` 为准：自身偏好、打开主窗、原生拖动、成长、互动、装备及 Agent 状态快照；另有事件监听权限。成长导出 / 导入 / 重置三个命令仅主窗口可用，星伴不得读取历史、图片或剪贴板，也不能自行申报工作完成奖励。`src-tauri/src/lib.rs` 含声明与生成后运行时 ACL 测试。

依赖与构建入口见 `package.json`、`rust-toolchain.toml`、`.github/workflows/verify.yml`。现有 UI 直接使用 Vue 与 CSS；新能力优先复用现有组件。

版本与分发：`scripts/version.mjs`、`scripts/release-manifest.mjs`、`CHANGELOG.md`、`.github/workflows/{verify,release}.yml`；固定 endpoint 与公开验签密钥位于 `tauri.conf.json`，私钥不得入仓库。见 [发布指南](../../../../docs/releases.md)。

Homebrew 同步独立使用 `.github/workflows/homebrew.yml`、`scripts/homebrew-release.mjs` 与 `tests/homebrew-release.test.ts`。读取公开 latest 稳定 Release，校验实际 DMG 与清单及应用身份后生成 Cask；仅末尾 Tap job 使用单仓 deploy key，推送前重查 Release，拒绝降级 / 同版换摘要。详见 [Homebrew 自动化](../../../../docs/homebrew-release.md)，不能把同步验包称为完整安装升级验收。

产品宣传素材源为 `scripts/share-posters.html`，直接渲染既有星芽组件与原创界面 / 代码示意；中文主题 ID 为 `overview` / `workflow` / `companion` / `creation` / `ai`，英文对应加 `-en`，均为 1080 × 1440。`scripts/compress-share-images.py` 按 `product-share-catalog.json` 的十个图片条目制作 256 色索引 PNG 并回填字节数，Pillow 仅为素材制作依赖；文案 ID 为 `short` / `detailed` 及其 `-en` 版本。素材语言由面板临时选择，不写应用偏好、不自动发布；macOS 英文 AI 图原生保存 / 取消已验收，复制 / 跨应用粘贴与 Windows 原生分享仍需单独验收。见 [分享说明](../../../../docs/product-share.md)。

可选统计：`telemetry.rs` 只接收 9 种固定事件并聚合计数，主窗只申报星绘坊打开，其余由原生操作路径计数。缺失偏好默认开启，已保存的关闭选择保留，读取 / 解析失败关闭；无有效接收端时不采集。复用 monitor 约每分钟 checkpoint / 调度发送，发送前后和正常 `RunEvent::Exit` 也 checkpoint；下载验签完成和进入安装阶段后另调度异步发送，不等待网络、不持有存储锁。固定缓存位于默认应用目录 `usage-counts.json`，读取上限 2 KiB，9 类各最多 100 次、TTL 72 小时，只存格式 / 应用版本、创建时间和计数；同版本恢复，换版本丢弃，不含设备 / sessionId，不加入剪贴备份或目录迁移。异常终止可能丢最后一分钟新增计数；安装只计尝试，补报按收到时间计，可能丢失或重复。

失败在本次运行退避 1 / 5 / 15 / 60 分钟，之后最长每小时一次，重启重置退避。关闭清队列 / 磁盘缓存并更换会话；清空递增 generation，使迟到的旧请求结果不能重新入队。`TelemetrySettings.vue` 与 `telemetry-api.ts` 展示配置、开关状态和白名单待发次数，事件按 revision 接收，操作中保护窗口释放；发送成功 / 网络失败不弹提示，用户主动修改偏好的本地保存错误仍需反馈。

当前 v2 载荷由 `telemetry_device.rs` 提供应用二进制架构、系统主版本、Mac 公开产品代号 / Windows 固定产品系列 / Linux 通用类别；客户端与 Worker 双重白名单，不传 serial / 主机名 / 用户名 / 永久 ID。设备数据不进入本地计数缓存。`telemetry-relay/` 源码兼容 v1 / v2、默认仅转发 GA4；既有 `desktop-usage-relay` 已更新为兼容 v1 / v2 的生产中继，账户已确认 Cloudflare Workers Free。2026-10-29 远端与本地 SHA-256 均为 `d127d1fb4b24f4cadff240ab0a4a29b90d371119358c0b3df304d106899b96e9`，日志 / 跟踪 / 问题开关及微软开关仍关闭；真实 Rust v2 固定 9 事件批次返回 204。GA `app_arch` / `os_major` / `device_model` 事件维度已创建并核对，Realtime 的 `app_started` 已显示这三个参数（每键 2 条），`device_model` 下钻显示公开 Mac 产品代号（2 条）；此实时证据独立于 HTTP 成功；后续 GA4 常规探索已核对固定范围的 14 条事件 / `Usage count` 合计 22，macOS 进程级断网缓存、正常重启恢复与恢复联网后清队列也已验收。长期持续观察、Windows 统计联网仍未完成，分阶段证据见 [验证记录](../../../../docs/verification.md)。Google 配置无效时不外发，密钥只放云端。Workers 实际运行不接受 `redirect: 'error'`，`send` 使用 `manual` 并拒绝 3xx；Worker 不重试，客户端重试可能重复计数。Application Insights 仅在 `ENABLE_APPLICATION_INSIGHTS=true` 时校验并启用，未设置或为 `false` 时忽略残留连接字符串、不请求 Azure；当前云配置为 `false`。Worker 名称可配置，其他产品以独立部署隔离，当前没有多产品路由。构建变量 `STARCLIP_TELEMETRY_ENDPOINT` 已在 GitHub 设置并回读核对；它是 Rust 编译期地址，不含供应商密钥，运行时赋值不能更改旧包。详见 `docs/telemetry.md`。
