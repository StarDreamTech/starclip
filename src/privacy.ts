import type { Clip } from './types';

export interface PrivacyRule { id: string; name: string; enabled: boolean; condition: 'keyword' | 'domain'; value: string; tag: string | null; markPrivate: boolean }
export interface PrivacyRules { revision: number; rules: PrivacyRule[] }
export interface PrivacyScan { token: string; total: number; privateCount: number; newlyPrivateCount: number; changedCount: number }
export const sourceNames: Record<string, string> = { wechat: '微信公众号', linuxdo: 'LINUX DO', x: 'X', 'x-short': 'X 短链 · 未解析', bilibili: '哔哩哔哩', 'bilibili-short': '哔哩哔哩短链 · 未解析' };
export const protectedClip = (clip: Clip) => !!clip.isPrivate || !!clip.privacyManual || !!clip.privacyReasons?.length;
export const pendingClip = (clip: Clip, revision: number) => !clip.scanVersion || clip.scanVersion !== revision;
// Describe the matching rule without echoing a secret or the matched value.
export function privacyReasonExplanation(reason: string): string {
  if (reason.startsWith('自定义规则：')) return '命中了自定义隐私规则的关键词或域名条件。';
  return ({
    '私钥标记': '检测到 PRIVATE KEY 私钥区块标记。',
    '私钥格式': '检测到 PRIVATE KEY 私钥区块标记。',
    '凭据格式': '检测到 API 密钥或访问令牌的常见前缀和长度特征。',
    '凭据字段': '检测到 password、token、api_key 等敏感字段及非空值。',
    '疑似凭据': '检测到密码字段或访问令牌格式。',
    '链接包含凭据': '链接的账号、查询参数或片段中包含凭据字段。',
    '敏感链接': '链接的账号、查询参数或片段中包含凭据字段。',
    '助记词格式（词表与校验通过）': '文字匹配助记词词表，且通过对应校验。',
    '疑似助记词（词表匹配，校验未通过）': '文字连续匹配助记词词表，未通过校验，需人工复核。',
    '内容超过扫描上限': '内容超过本地扫描上限，尚不能完成判断。',
    '链接数量超过扫描上限': '链接超过单次扫描上限，尚不能完成判断。',
    '链接解析未完成': '部分链接无法解析，尚不能完成判断。',
    '复制来源标记为敏感内容': '复制它的应用（如密码管理器）通过系统约定声明这是敏感或临时内容。',
  } as Record<string, string>)[reason] ?? '已保留的隐私依据；可在本机复核后尝试解除。';
}
export function originHost(clip: Clip): string {
  try { return clip.originUrl ? new URL(clip.originUrl).hostname.toLowerCase() : '' } catch { return '' }
}
export function matchesMetadata(clip: Clip, sources: string[], tag: string, privacy: string, revision: number): boolean {
  return (!sources.length || sources.some(source => source.startsWith('host:') ? clip.linkHosts?.includes(source.slice(5)) || originHost(clip) === source.slice(5) : clip.linkSources?.includes(source)))
    && (!tag || !!clip.tags?.includes(tag))
    && (privacy === 'all' || (privacy === 'private' ? protectedClip(clip) : privacy === 'pending' ? pendingClip(clip, revision) : !protectedClip(clip) && !pendingClip(clip, revision)));
}
export function normalizeTags(input: string): string[] { return [...new Set(input.split(/[,，\n]/).map(value => value.trim()).filter(Boolean))]; }

// Browser fixtures only. Desktop policy and complete sensitive detection live in Rust.
export function analyzeDemoClip(clip: Clip, config: PrivacyRules): Clip {
  const hosts = new Set<string>(), sources = new Set<string>(), reasons = new Set((clip.privacyReasons ?? []).filter(reason => clip.kind !== 'image' || reason !== '图片尚未进行文字识别')), tags = new Set(clip.manualTags ?? clip.tags ?? []);
  const content = `${clip.title}\n${clip.kind === 'image' ? '' : clip.content}`;
  for (const raw of content.match(/https?:\/\/[^\s<>"']+/gi) ?? []) {
    try {
      const url = new URL(raw.replace(/[),.;，。；）]+$/, '')), host = url.hostname.toLowerCase().replace(/\.$/, '');
      hosts.add(host);
      const domain = (value: string) => host === value || host.endsWith(`.${value}`);
      if (host === 'mp.weixin.qq.com') sources.add('wechat');
      if (domain('linux.do')) sources.add('linuxdo');
      if (domain('x.com') || domain('twitter.com')) sources.add('x');
      if (host === 't.co') sources.add('x-short');
      if (domain('bilibili.com')) sources.add('bilibili');
      if (host === 'b23.tv') sources.add('bilibili-short');
      if (url.username || url.password || [...url.searchParams.keys()].some(key => /^(access_token|password|secret|api_key|token)$/i.test(key))) reasons.add('敏感链接');
    } catch { /* Invalid URLs are not classified. */ }
  }
  if (/-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/.test(content)) reasons.add('私钥格式');
  if (/(?:password|api_key|secret)\s*[:=]\s*\S{8,}|Bearer\s+[\w.-]{12,}/i.test(content)) reasons.add('疑似凭据');
  for (const rule of config.rules.filter(rule => rule.enabled)) {
    if (rule.condition === 'keyword' ? content.toLocaleLowerCase().includes(rule.value.toLocaleLowerCase()) : hosts.has(rule.value.toLowerCase().replace(/\.$/, ''))) {
      if (rule.tag) tags.add(rule.tag);
      if (rule.markPrivate) reasons.add(`自定义规则：${rule.name}`);
    }
  }
  return { ...clip, privacyManual: !!clip.privacyManual, isPrivate: !!clip.privacyManual || reasons.size > 0, privacyReasons: [...reasons], scanVersion: clip.kind === 'image' ? 0 : config.revision, tags: [...tags], manualTags: [...(clip.manualTags ?? clip.tags ?? [])], linkHosts: [...hosts], linkSources: [...sources] };
}
