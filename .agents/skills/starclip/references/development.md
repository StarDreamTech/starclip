# 开发验证、踩坑与待办

## 当前状态与验证快照（2026-10-29）

- [0.2.3](https://github.com/cpython666/starclip/releases/tag/v0.2.3) 已按用户授权于 2026-10-29 14:55:38 UTC 正式公开为 latest（Release `427924020`，非草稿、非预发布）。`v0.2.3` 指向 `45ffe2532de3da87a844b25eff6f64ca173f5ca1`；[Release 40882819215](https://github.com/cpython666/starclip/actions/runs/40882819215) 8 个 job 全成功，有 8 项完整附件。Homebrew [40896366521](https://github.com/cpython666/starclip/actions/runs/40896366521) 两个 job 全成功，Tap 提交 `79d2e48ca567086a5e1b15863787a8810490b6bd` 的 Cask 已读回 0.2.3 / 正确 DMG SHA；本机正式安装仍为 0.2.2。签名 DMG 7,603,121 B、macOS 更新归档 7,796,156 B、Windows NSIS 6,354,060 B；全部 8 项公开附件已匿名下载，其大小 / SHA-256 与已验收草稿、GitHub digest 和 `SHA256SUMS` 一致；稳定 latest.json 完整字节相同、version=0.2.3。Windows 公开下载曾超时一次，重试后核对通过。详细摘要与证据只记在 [发布与原生验收](../../../../docs/verification.md#023-正式发布与原生升级验收)，原包在忽略的 `artifacts/release-0.2.3.noindex/signed-assets/`。
- Windows [40891766297](https://github.com/cpython666/starclip/actions/runs/40891766297) 已从**真实公开 0.2.2** 经原生 UI 检查、签名下载、稍后、确认安装及自动重启到该精确 0.2.3；目标 EXE 与签名 NSIS 内字节一致。暂停、原创 PNG / 缩略图、记录 / 分组、成长 / 装备及固定偏好保留，统计关闭；恰好 1 个 before-update 快照和旧 EXE 备份均核对，cleanup 完整恢复。macOS 最终 [40895064653](https://github.com/cpython666/starclip/actions/runs/40895064653) 的 job 已成功，同目标签名安装、固定 PNG / 成长偏好与恢复副本通过；历史 0.2.2 仍需手动补开，结果是 `passed-with-manual-recovery`，不算自动重启通过。最终清理确认信任撤销、证书 / 私钥删除、hosts 恢复及服务停止，正常 TLS 复测 curl 60；本轮 CA 的 deny 记录随一次性 VM 销毁，不宣称信任库逐字节恢复。第二轮 `40889704576` 核心通过但清理超时的历史故障保留在验证记录。
- 两平台草稿验收使用一次性 runner 内临时受信任 CA / hosts 提供**原始签名包和正式 HTTPS URL**，未关闭 TLS / 主机名 / 签名校验；原生 UI 流程未在公网 CDN 上重跑，公开附件匿名下载和清单一致性是顶部单独记录的证据。Windows 当前用户 Root 导入曾阻塞，改为已提升权限的 disposable runner 机器根存储、限时 certutil 和精确 thumbprint 删除后通过；只在该 runner 使用，不用于用户电脑。共享 helper 的所有文本读写须显式 UTF-8，避免 Windows cp1252 解析中文清单失败。入口为 `release-updater-fixture.py`、`windows-updater-acceptance.yml` / `.ps1`；证据目录和边界见 [验证记录](../../../../docs/verification.md#023-正式发布与原生升级验收)。
- 签名前最终候选 `841431d` 的 [Verify 40873603974](https://github.com/cpython666/starclip/actions/runs/40873603974) 7 个 job 全通过；其 7,618,896 / 6,350,661 B 安装包与更早 `8c95044` 测量包均保留作历史，不与上述签名产物混用。宣传「macOS 安装包不到 8 MB」仍成立，不指运行内存。分享 4 段文案 / 10 张 PNG 共 805,747 B，Node 186、Web、Rust share 2 项通过；macOS 独立空库英文 AI 图保存 / 中文取消、浏览器切语种保留主题已核对，未测系统图片剪贴板或 Windows 原生分享。固定阴影实验同包单轮 120 秒 201.3 / 199.6 MiB，1.7 MiB 差异未证明稳定收益，不合入；详情见 [候选与分享](../../../../docs/verification.md#023-候选整理与分享素材)。
- 启动偏好复核发现原生托盘同步调用不能持有存储锁，否则与主线程暂停回调形成死锁。`desktop::apply_startup_preferences` 使用独立操作锁、缩短存储临界区并在保存前重查冻结；正常 / 冻结回滚定向测试通过。扩展的 Windows 启动设置 UI smoke 已在 `f3da89d` 的 fresh / upgrade 两组 CI 原生通过。macOS 完整更新已在 `22480ca` 修复重启退出顺序后通过，源码与实测边界见本页更新快照及 [完整链路](../../../../docs/verification.md#macos-应用内更新完整链路)。
- 本机正式 0.2.2 二进制缺少 `get_main_window_close_request` / `get_clips`；不能由历史测试包相同的版本文字推断包含新代码。本轮未替换本机正式应用，发布状态以顶部正式发布条目为准。
- 候选提交 `f3da89d45fb25f46d88130794b3cfe77aa7786f5` 的 [Verify 40852732856](https://github.com/cpython666/starclip/actions/runs/40852732856) 全部成功：Node 183、core 112（另 3 ignored）、Web 构建；macOS desktop 128 / ai_stdio 1、Windows desktop 123 / ai_stdio 1（两平台 lib 各另 3 ignored），以及各自成品 CLI / MCP 1 项、23 款字体实际下载校验 / 离线渲染、app / DMG / NSIS 与全新安装 / 真实 0.1.0 覆盖升级均通过。Windows fresh / upgrade 原生 UI 还确认 HKCU 登录项启停及外部禁用回读、隐藏冷启动不创建主 HWND、二次启动恢复、托盘偏好持久化和隐藏图标后设置内退出，始终暂停、固定数据保留。未覆盖实际注销登录、Explorer 图标显示 / 点击或 macOS 应用内更新安装 / 重启。该结果只对应此提交，后续工作区修改仍需重新验证；更早 `7fbbae5` 的 CI 证据保留在 [历史候选 CI](../../../../docs/verification.md#双平台候选-ci)。该轮未操作正式安装或发布。
- 既有隔离原生证据：123 条原创文本、始终暂停，50 条分页、第二页与全库搜索、隐私变更后的详情保护、26 种日常表情及静默发送已核对。清理预览显示前 100 条、取消不删记录、重开不保留旧确认已验收，没有在原生执行清理删除。
- macOS 内存诊断已完成：空库、暂停、100% idle 星伴、成长关闭，主控制页销毁后剩 1 个 WebContent，固定释放后 30 / 120 秒采样。首包 13.56 MiB 实测空白 1 轮、静态 2 轮、动画 3 轮，动画 120 秒中位 196.0 MiB、GPU 约 119 MiB；新包 13.58 MiB 的同包动画基线 194.0 MiB、共享 `defs` + `use` 单动态轮廓实验 192.5 MiB，1.5 MiB 差异小于首包动画复测波动，未证明稳定收益，实验不合入生产。两包构建及 strict / deep ad-hoc 验签通过；新包资源重新分块，不跨包计算实验收益。空白 / 静态仅为诊断对照；诊断前端不等于生产前端 GUI 验收，不与旧 123 条文本的 242.6 / 36.7 MiB 样本计算降幅，动画几十 MiB 目标仍未达到。逐轮数据见 [内存诊断与共享路径实验](../../../../docs/verification.md#内存诊断与共享路径实验)。
- 统计兼容 v1 / v2 中继与 Realtime 设备参数已核对；2026-10-29 GA4 非实时探索确认 2026-10-01 至 10-28 固定范围内 9 类事件 14 条、`Usage count` 22 次。专用空库、暂停采集的 macOS 包已完成进程级断网缓存、正常退出 / 重启恢复、联网后清队列与静默发送。只限制测试进程出站，不覆盖系统网络切换、Windows 原生断网或长期持续观察。证据集中在 [统计说明](../../../../docs/telemetry.md)，不复制账号、私有报表网址或具体硬件代号。
- 字体目录为原 9 款 + 外部 14 项共 23 项，均已有隔离摘要 / 中文覆盖 / 离线渲染检查。原 9 项按用户确认统一显示「来源于互联网」，外部作者署名保留；本轮字体 / 预览 / i18n 定向 15 项通过。macOS 已完成原「手写体」代表项在线下载 / 预览 / PNG 导出，以及外部「站酷快乐体」下载后断网重启、本地预览与 1200×1600 PNG 导出；两平台 CI 已验证 23 项逐款实际下载、摘要和离线渲染，仍不扩大为 23 项逐款 GUI 或 Windows 字体 GUI 验收。
- 历史更新证据保留：macOS `22480ca` / [40862701839](https://github.com/cpython666/starclip/actions/runs/40862701839) 与 Windows `7fbbae5` 曾用源码覆盖比较版本 0.2.1，完整更新到固定公开 0.2.2；它们证明对应更新实现，并非真实历史 0.2.1 二进制升级。真实公开 0.2.2 → 新签名 0.2.3 的独立结果已记在顶部，不再将旧实验当作新目标证据。历史详情见 [macOS](../../../../docs/verification.md#macos-应用内更新完整链路) / [Windows](../../../../docs/verification.md#windows-应用内更新完整链路)。
- 各能力历史版本、样本和日期以 [验证记录](../../../../docs/verification.md) 为准，不把旧验收扩大成最新全功能验收；候选 CI 只证明所列提交与场景，后续隔离实验须独立验证。

- 2026-10-29 先前 macOS 启动设置隔离验收：独立空库暂停验证登录项启停、系统侧禁用回读、LaunchAgent bootstrap 启动、隐藏主窗和再次呼出、图标即时设置及退出草稿保护。临时诊断确认隐藏首启不建主窗，CUA 激活才发 Reopen；诊断已撤回，证据留忽略目录 `artifacts/acceptance.noindex/startup-check/`。`launchctl print-disabled` 需兼容 enabled/disabled 和 true/false；Windows 本轮构建及原生启动设置随后已在上述 CI 通过，两平台真实注销重登仍未验证。测试服务 / 登录项已撤销，测试 app 已退出并移入废纸篓；正式安装未更改，详见 [启动验收](../../../../docs/verification.md#未发布启动与后台运行2026-10-29)。

- 2026-10-29 先前工作区 release 模式测量：Apple Silicon DMG 6,978,352 B（6.98 MB）、app 文件 15,013,697 B（15.01 MB）；独立测量 ID，不运行、不安装、不发布，深度 / 严格 ad-hoc 验签通过，不能等同公证或 updater 签名。字体缓存 / 用户数据另计，47 MiB 调试包不能作为正式模式包体积。测量 app 已注销后移入废纸篓，DMG 和结果在 `artifacts/package-size-20261029.noindex/`；Windows 新候选 NSIS 构建已通过，本条未核对其包体字节数，见验证记录。

## 环境与检查

- 字体下载复用 reqwest 0.13 系统代理与 TLS；rustls-no-provider 构建客户端前必须安装 ring provider，已有 provider 时保留。真实测试发现遗漏会 panic，不可改成绕过 TLS。GitHub 连接 10 秒 / 总计 60 秒超时，失败不安装半文件。

内存测量先辨认正在运行的可执行文件、构建时间和关键接口；开发源码、隔离测试包、正式安装与线上 Release 是不同对象。用 `launchctl print pid/<pid>` 的应用域 / bundle ID 核实辅助进程，再用 `footprint --noCategories --format bytes` 汇总；不把系统所有 WebKit 相加，也不只量 Rust 主进程。RSS、虚拟地址空间、安装包大小与物理足迹不可混用。

复测固定数据量、采集状态、主窗口 / 草稿状态、星伴大小与心情，分别记录打开、关主窗、收起星伴后的状态和等待时长。关窗后先用进程工具测量，CUA 读取可能激活 / 重建页面；确认页面退出与验证下降数值分开。短时波动不能证明泄漏或长期稳定。主窗早期关闭、busy 结束续办、设置子页草稿与重开令牌由 `window-release` / `settings-release` 回归保护；原生只把实际做过的场景记为通过。

依据 `package.json`、`rust-toolchain.toml`、`src-tauri/Cargo.toml` 和 `.github/workflows/verify.yml`：Node.js 22.12+，Rust 1.90.0，Tauri 2。浏览器开发命令 `npm run dev` 固定在本机 1420 端口；桌面 `npm run tauri dev` 会启动原生应用，不能当作纯编译检查。

有行为变化时执行相关前端和 Rust 基础检查：

```sh
npm test
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features --lib
```

Rust 修改检查格式；桌面改动还要检查 desktop 特性和平台打包。权限或窗口相关改动运行 desktop lib 测试（其中含生成后的运行时 ACL 验证，不创建应用或访问真实剪贴板）：

```sh
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
```

平台打包用 `npm run tauri build`；macOS 可用 `npm run tauri -- build --bundles app,dmg`。首次缺少生成图标时用 `npm run tauri icon public/starclip.svg`。CI 配置为 Apple Silicon macOS app/DMG 与 Windows x64 NSIS；是否成功须查对应运行，不能仅凭 workflow 推断。磁盘不足用 GitHub Actions 产物，不清理其他项目或个人文件。

仅修改文档/skill 时核对事实、链接、结构和 `git diff --check`，不把未运行的构建/实测写为本轮通过。测试数量和包大小是历史构建快照，不作为恒定项目属性。

## 回归入口

| 修改主题 | 优先查阅的测试 |
| --- | --- |
| 主题、颜色、偏好兼容 | `tests/theme.test.ts`、`tests/color.test.ts`、`tests/api-import.test.ts`、`store/` 内测试 |
| 界面语言、文案回退 | `tests/i18n.test.ts`；改文案或数据名称后同时核对 `src/i18n/en*.ts` 与 `hasTranslation` 覆盖测试 |
| 自定义配色与本地贴纸 | `tests/appearance.test.ts`、`tests/appearance-panel.test.ts`，Rust `appearance.rs` / `store/` / ACL；原生使用原创 PNG、隔离应用标识 |
| 导出反馈、监听释放 | `tests/backup-export.test.ts` |
| 清理、图片、备份、迁移 | `tests/cleanup.test.ts`、`tests/settings-release.test.ts` 的预览释放 / 迟到响应、`tests/image-clip.test.ts`、`tests/api-import.test.ts`，Rust `store/` / `images.rs` / `storage.rs` / `capture.rs` |
| 卡片排版、手写异步结果 | `tests/code-card.test.ts`、`tests/snippets.test.ts`、`tests/handwriting-preview.test.ts`、`tests/handwriting-fonts.test.ts`、Rust `handwriting.rs` |
| 隐私、来源、标签与历史扫描 | `tests/privacy.test.ts`、`tests/privacy-panels.test.ts`、v3 备份夹具；Rust `privacy.rs` / `store/` / `storage.rs`，使用原创或公开 BIP-39 测试向量 |
| AI 授权与官方接口 | `tests/ai-api.test.ts`、`tests/privacy-panels.test.ts`；Rust `ai_access.rs`、`ai_bridge.rs`、`src-tauri/tests/ai_stdio.rs`，检查真实主程序 CLI / MCP stdio 与权限一致性 |
| 产品分享 | `tests/product-share-api.test.ts`、`tests/product-share-panel.test.ts`，Rust `share.rs` / `desktop/` / ACL；资产元数据必须与 `public/share/*.png` 一致 |
| 更新、恢复、版本发布 | `tests/update*.test.ts`、`tests/release.test.ts`，Rust `updater.rs` / `storage.rs` / `store/` / ACL |
| 可选使用统计与接收端 | `tests/telemetry-settings.test.ts`、`tests/telemetry-relay.test.ts`，Rust `telemetry.rs` / `store/`；真实 sender 检查默认忽略，显式运行步骤及原生 / GA 验收边界见 `docs/telemetry.md` |
| 星伴表情、大小与互动 | `tests/starpet*.test.ts`、`src-tauri/src/lib.rs` / `store/` / `desktop/` |
| 主窗安全释放、重建与草稿保护 | `tests/window-release.test.ts` / `tests/settings-release.test.ts` 运行真实 App / 子页 setup 与 IPC，`desktop/` 请求编号回归；原生核实 WebContent 退出及草稿保留 |

互动菜单回归集中在 `tests/starpet-desktop-menu.test.ts`：真实 Vue 组件的内存渲染、外部点击 / 焦点 / 显隐、触发器切换、延迟动作、监听释放，以及原生失焦事件的窗口范围。该测试不访问系统剪贴板，也不等于跨应用原生点击实测。

更新回归覆盖默认开启、独立保存、24 小时间隔（含失败 / 重启 / 时钟回退）、关闭排队检查、稳定版本过滤、ready 包保护，以及真实 Vue 提示的延迟响应、忽略单版本与忙碌释放；参见 `tests/update-api.test.ts`、`tests/update-settings.test.ts`、`store/` / `updater.rs`。图片隐私回归使用原创图片与显式注入的旧 JSON 原因，验证读取不改数据库、旧备份导入、手动与真实规则保留、图片仍不可授予 AI，详情展示匹配依据不泄漏原文。

隐私面板回归验证规则保存失败保留草稿、预览后显式扫描，AI 面板验证排除隐私 / 待扫 / 图片、明确 ID 与字段、重复提交、失败和数据变化使预览失效。`api.ts` 新增运行时导入时，同步更新以 data URL 转译加载该文件的既有测试映射；浏览器内存模拟不能替代 Rust 检测或原生授权验收。

当前手写内置化修改已通过自动检查与隔离渲染：覆盖字体下载校验与失败不启用、离线渲染、首次不下载、静态预览及过期结果隔离。目录已扩到 23 款字体与字重，清单检查保证固定版本来源、大小上限、唯一 ID、作者与标签、小预览及按来源分别提供的许可 / 提供说明；全部真实字体已用隔离目录验证 SHA-256、中文字符覆盖及离线 CPU 渲染。`storage.rs` 使用合成字体字节验证 `current/fonts` 的占用、迁移、快照与恢复，并拒绝根目录 / 子路径符号链接；普通测试不需要真实 TTF 或个人数据。原有两款已有隔离原生下载 / 预览 / 导出证据；2026-10-29 追加原「手写体」代表项在线流程与外部「站酷快乐体」下载后断网重启 / 本地预览 / PNG 导出，23 项仍未逐款操作原生 GUI。不要为凑数运行真实应用或读取剪贴板。

暂停偏好回归见 `store::save_settings_with_pause_guard`、`tests/api-import.test.ts` 与设置相关组件检查：`pauseRevision` 旧数据默认 0，Store 在事务内比较期望版本并只在 `paused` 实际变化时递增，拒绝旧草稿覆盖较新暂停选择，含 ABA 变化；前端刷新只同步暂停字段与版本，保留其他草稿，版本不计入 dirty。当前源码自动检查已通过，不能据此声称使用真实剪贴板完成原生采集验收。

## 已踩过的坑

- 2026-10-29 核查 macOS 应用搜索中的重复项：`mdfind` 及各 `Contents/Info.plist` 确认当时共 30 个 Starclip 包，其中正式安装 1 个、`src-tauri/target/release/bundle/macos/` 构建 / 隔离测试包 20 个、`artifacts/` 历史产物 7 个、Application Support 中旧应用备份 2 个。隔离测试包的用途见 `docs/verification.md`，保留的 `.app` 被系统索引导致搜索拥挤。随后按用户要求，将 27 个未运行、未被 Git 跟踪的开发产物逐路径取消 Launch Services 注册并通过系统废纸篓接口移走；已核验原路径消失、27 个废纸篓目标存在，搜索索引仅余 `/Applications/Starclip.app` 和两份升级备份。本轮验收新建的最后两个 `Starclip Acceptance.app` / `Starclip Memory Diagnostic.app` 也已退出、逐路径取消注册并移到废纸篓，原路径消失、Spotlight 不再返回本轮测试包；清理证据见 `artifacts/acceptance.noindex/final-app-cleanup.txt`。正式包 / 备份的 `Info.plist` 摘要始终与最初清单一致，未操作数据目录；历史测试包路径现为验收出处，不能再当作可直接启动的当前产物。后续验收收尾应处理不再需要的测试应用包，先区分正式安装、测试产物和恢复用备份，不能按 Starclip 名称一概删除。
- 首次关闭后不能立即用 CUA 读取应用来测后台：读取可能激活 / 重开主窗口，应先以进程工具采样。去滤镜 A/B 无确认收益已恢复；多副本实验意外开启了其中一个隔离副本采集，停止并恢复暂停，未查看新增图片，该实验作废。后续一次只运行一个验收副本，并核对暂停状态；不能把异常实验写成未访问剪贴板的通过项。
- 原生 dialog 顶层会遮住外部 toast：导出结果放在设置内，通用提示归当前活动 dialog。整理弹窗不能替换/销毁父设置，Escape 与保存快捷键要遵循当前层级。
- 即时主题和星伴更新不能调用普通偏好保存，否则会带入草稿或清理副作用；快速连续写入需顺序合并、失败恢复最后确认值，跨窗口采用补丁合并。
- 独立保存的 checkbox / number 原生 DOM 可能先改变而 Vue 状态仍为旧值；失败回退需同步 DOM，不能只依赖「赋回相同 ref」。外观组件已覆盖。自定义色需要同时考虑强调色文字、按钮前景和受渐变影响的阅读底色；删除覆盖变量才能完整恢复主题默认。
- macOS 原生拖动要在按下时直接发起，等待移动阈值/长按或异步加载会错过起拖时机；命令成功或入口存在仍不等于实际位移验收。
- 原生截图可能被工具扁平为 RGB 白底，不能证明透明窗口真实透显；数值运动测试不能证明流畅度或长期 CPU 占用。
- macOS Dock 重新打开需处理 `RunEvent::Reopen`；`has_visible_windows` 包含星伴浮窗，不能据此跳过主窗显示。2026-10-26 已接入现有 `show_window`。CUA 本轮无法读取 Dock 控件，且关闭后读目标状态无法可靠区分工具激活，真实 Dock 点击 / 最小化 / 仅星伴可见场景仍待人工确认；隔离原生默认不限条数和天数已核对。依据见当日验证记录。
- 2026-10-26 默认 Tauri 打包的整包签名核验出现「code has no resources but signature indicates they must be present」。通过 `tauri bundle --bundles app,dmg --config '{"bundle":{"macOS":{"signingIdentity":"-"}}}'` 重新做本地 ad-hoc 签名，两处 app（产物及 DMG 内）通过 strict/deep 核验；不等于公证。DMG 验签挂载点放在开发服务器监听目录之外，或先停 Vite，避免其沿安装器的 Applications 链接扫描其他应用资源。
- 不把编译、浏览器演示或某次 macOS 场景通过扩大为 Windows、多屏、全屏或全功能验收。历史安装包、本地产物与早期 alpha.1 发布附件可能不同，交付时核对对象。

## 历史验收的使用范围

既有 macOS 隔离验收包括隐私与 AI 连接、清理保护、迁移 / 恢复、首启教程、图片采集 / 复制、主题、星伴与成长存档等；Windows 与 macOS 的全新安装 / 真实 0.1.0 覆盖升级已在固定数据 CI 中通过。它们证明各自版本和场景，不证明每次修改都已复测；两平台应用内更新 GUI 完整链路均有独立证据，源码提交与固定公开目标见本页快照，不扩大为未来新目标验收。历史人工安装时做过一致快照及数据摘要核对；这是该次升级操作，不是后续任务的安装授权。详细历史只查 [验证记录](../../../../docs/verification.md)，不在知识库复制每轮测试计数。

## 产品分享验证边界

当前分享入口为设置 → 分享；源码 `ProductShare.vue`、`product-share.ts`、`share.rs` 实现固定文案 / 图片复制及专用保存命令。中英文各有两类文案、五个图片主题，10 张 1080 × 1440 索引 PNG 合计 805,747 字节；素材源、重制和压缩流程见 [分享说明](../../../../docs/product-share.md)。Python / Pillow 仅用于制作素材，不增加运行或正常构建依赖。最新候选原生代表图保存 / 取消见本页快照；下段保留历史版本证据，不能代替当前素材验收。

Rust 纯目录 / PNG 校验、模拟成功 / 失败写入的自采集抑制、生成后的窗口 ACL 已检查；浏览器复制测试采用模拟接口，不读取真实剪贴板。不要把模拟复制成功写成系统剪贴板 / 第三方粘贴实测。2026-10-26 独立 `com.starclip.share-verify` 原生实测保存实际路径与取消，保存 PNG 摘要与源文件一致；Node 58 / Rust core 54 / desktop lib 58 项通过（Rust 各 1 项忽略）。最终分享版 app / DMG / 签名 updater 归档位于忽略的 `artifacts/product-share-0.2.0/`，本地 app / DMG 内应用及更新签名核验通过，未替换正式安装。具体结果与边界由 [验证记录](../../../../docs/verification.md) 维护，不沿用此前升级版验收。

Tauri 资产解析器在正式包中读取嵌入资产，在带 `devUrl` 的开发模式回退到 `dist/`；修改 PNG / catalog 后先跑 `npm run build`，否则旧尺寸或字节数会导致校验失败。PNG 被复制为系统图片像素，接收方可能重编码变大；实际小文件分发使用「保存图片」。

## 发布、Homebrew 与更新排障

- GitHub Releases 存实际安装包与签名清单，自有 `cpython666/homebrew-tap` 存 Cask；无需独立 Homebrew 账号。已公开 0.2.2 为 Apple Silicon macOS / Windows x64；Tap 仅提供 Apple Silicon，非官方主仓收录。SourceForge / Codeberg 仅是历史候选，未接更新回退；WinGet 未上架，PyPI 不是任意桌面包附件仓库。平台额度会变化，对外建议需重新核查。
- `scripts/version.mjs` 统一 npm / Cargo / Tauri 六处版本，`CHANGELOG.md` 提供发布说明；`release.yml` 验证双平台构建、CLI / MCP、全新安装及旧版覆盖后上传完整草稿，公开需用户确认。详细步骤见 [发布指南](../../../../docs/releases.md)；验证失败不移动旧 tag 或把未完成附件设为 latest。
- `homebrew.yml` / `homebrew-release.mjs` 仅接受公开 latest 稳定版，核验实际 DMG 大小 / SHA-256 / GitHub digest、签名、版本、标识与架构，再执行 style / audit / fetch；专用单仓 deploy key 最后非强推 Tap。拒绝降级和同版换包，推送前重查资产；`GITHUB_TOKEN` 发布需显式 dispatch 下游。0.2.2 的 [同步运行](https://github.com/cpython666/starclip/actions/runs/40466604899) 已完成实包验证与远端 Cask 更新，不是单纯 dry run。见 [同步说明](../../../../docs/homebrew-release.md)。
- Homebrew 检查在 Tap 上下文运行，普通 Ruby 文件会套用错误规则；历史 CI 通过更新 Homebrew、限定信任该 Cask、使用现有只读 `github.token` 解决 style / trust / 匿名 API 限流。没有新增 PAT 或修改本机代理 / 信任。原更新签名私钥与 Tap deploy key 分离；密钥只在仓库外或云端 secret，不将签名资源写入 skill。
- 程序与数据目录独立，更新保持 `com.starclip.desktop`。无更新器的旧版第一次需覆盖程序，不删除数据；启动仍执行已保存保留规则。schema 2 升级先快照，事务迁移失败回滚；恢复复制到新目录，冻结写入待重启；旧应用副本不是自动二进制降级。流程与格式边界见 [数据安全](data-safety.md)。
- 下载与安装分开：只下载验签进入同会话 `ready`，用户确认后保存恢复点 / 旧应用，再安装并重启；没有跨进程下载缓存或断点续传。Windows 安装器启动前退出清理后发生失败要保持冻结 / `restart-required`，不能当普通下载失败解冻。独立 NSIS 覆盖升级通过不等于应用内按钮链路通过。
- macOS 同步 `desktop::restart_app` 不能直接调用 `app.restart()`：事件循环上的直接重启会跳过正常 `Exit`，单实例 socket 来不及清理，可能把替换后的新进程当成重复启动。`22480ca` 改为 `request_restart()` 后返回，由正常退出先清理；对照 run 40860315562 已替换正确目标却无新进程，修复后的 40862701839 自动新 PID / 目标摘要 / 暂停和快照全部通过。首轮 AX 只见窗口外框是测试助手问题，通过激活自有 app 并读取按钮子静态文本解决；runner 原已授权 Accessibility，没有修改 TCC。详情见 [macOS 完整链路](../../../../docs/verification.md#macos-应用内更新完整链路)。
- 检查更新失败先区分网络、HTTP 状态、发布清单是否存在与格式 / 签名，不能直接归咎 Clash。历史 latest.json 404 是当时无稳定 Release 附件，0.2.2 发布后经原有代理下载已为 200；这不是现在网络状态的永久保证。Tauri updater 的 macOS `system-proxy` 会读系统代理，GUI 未继承 shell 环境不是未走代理的充分证据；按错误枚举生成中文并标明 GitHub，不将丢失的 HTTP 状态猜成 404。
- 双平台旧版迁移夹具只写旧版共有字段，让应用真正迁移，不在 seed 脚本提前补 schema。macOS accepted socket 曾继承非阻塞状态，broker 已显式恢复阻塞读取并保留超时 / 帧上限；修复用真实分帧测试验证，不能删掉安全约束绕过测试。
- Worker 出站使用 `redirect: 'manual'` 并拒绝 3xx，运行时不接受 `redirect: 'error'`。`STARCLIP_TELEMETRY_ENDPOINT` 是 Rust 编译期变量，给旧包设置运行时环境变量不会更新地址；供应商 secret 仅在 Worker，其他产品用可配置名称的独立部署隔离。不要为复测触发发布流水线或改付费计划。

## 未完成与后续方向

- 动画星伴几十 MiB 目标与长期 / 大规模历史内存性能；Windows 统计真实联网 / 断网和字体 GUI 验收；双机完整环境迁移；星伴精确拖动、透显、多屏 / 跨全屏；托盘实际点击、快捷键冲突、休眠恢复。
- GA 长期持续观察与 Windows 原生断网；固定范围 `count` 汇总和 macOS 进程级断网 / 重启 / 恢复联网已验收，不承诺永久存储。23 项字体逐款 GUI、图片 OCR、规则试跑 / 扫描暂停续扫等见产品决策；原 9 项来源展示已按用户确认修正。分享 AI / 隐私主题与中英文素材已实现，系统图片剪贴板、跨应用粘贴与 Windows 原生分享仍待验收。
- 文件历史、GIF、富文本、自动粘贴、云同步 / 账号 / 剪贴内容公开链接、代码执行与内置模型分析尚未实现；已有只读 AI 接口不等于内置模型。稳定版提醒已实现，预览通道未开放，不承诺 LTS 维护年限。
- 无 Apple 公证或 Windows 发行者代码签名；本地 ad-hoc 与 Tauri 更新签名不等于系统发行者认证。待办不代表排期，后续新方案标「待定」。
