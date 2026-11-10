# Markdown 转图片（开发分支，未发布）

入口：主界面「Markdown 图片」直接打开星绘坊的 Markdown 编辑界面，与「代码卡片」「手写纸笺」并列，无需先进入工坊再选类型。主题以三个按钮直接展示；来源许可移至导出区下方。新建 Markdown 标题默认留空，正文使用 Markdown 样例。支持标题、列表、引用、表格、强调与代码块，正文在三种卡片之间共享，切换不改写原文。标题可清空，仅使用 Markdown 自身标题。

提供 540 / 720 / 960 px 宽度、14–28 px 字号、2 倍 PNG 文件导出。内置素纸、星紫、深夜三款，分别适配 Drake 的 drake-light、drake-purple、drake-dark。来源提交和 MIT 许可位于 `src/markdown-themes/`，界面可查看版权与许可全文。移除外部字体引用，使用系统字体；通过 Shadow DOM 隔离主题，只在打开标签时加载模块。

当前图片显示替代文字，HTML 按原文显示，链接不跳转。不支持公式、Mermaid、自动分页、图片复制、自定义主题导入或独立 Markdown 草稿保存。正文最多 30,000 字符 / 1,000 行，导出最多 2,400 万像素、高 16,384 px，超限提示拆分，不静默裁切。

实现：`src/components/MarkdownPanel.vue`、`src/markdown-render.ts`，PNG 复用现有 `api.saveExport`。

验证：前端构建通过；Markdown 安全渲染 / 尺寸限制、代码卡、手写预览及多语言共 16 项定向回归通过。Chromium 原创中文样例已导出三款主题的 PNG，浅色实图检查覆盖标题、引用、列表、表格与代码块。大文本浏览器输入验收遇到工具超时，不记为通过。尚未做 macOS WebKit / Windows WebView 原生导出验收，浏览器成功不代表平台验收完成。
