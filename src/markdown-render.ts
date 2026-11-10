import MarkdownIt from 'markdown-it';
const parser = new MarkdownIt({ html: false, linkify: false, typographer: false });
// No remote/local resources or active links are loaded from pasted documents.
parser.renderer.rules.image = (tokens, index) => parser.utils.escapeHtml(`[${tokens[index]!.content}]`);
parser.renderer.rules.link_open = () => '<span class="md-link">';
parser.renderer.rules.link_close = () => '</span>';
export function renderMarkdown(text: string): string {
  if (text.length > 30000) throw new Error('Markdown 最多 30,000 字符');
  if (text.split('\n').length > 1000) throw new Error('内容过长，请拆分后导出');
  return parser.render(text);
}
export function checkMarkdownImageSize(width: number, height: number): void {
  if (!Number.isFinite(height) || height <= 0 || width * height * 4 > 24000000 || height * 2 > 16384) {
    throw new Error('内容过长，请拆分后导出');
  }
}
