import { errorText, formatNumber, t } from './i18n/index.ts';
import type { UpdateStatus } from './update-api';

export function updateInProgress(state: UpdateStatus): boolean {
  return ['checking', 'downloading', 'backing-up', 'installing'].includes(state.phase);
}
export function downloadProgress(state: UpdateStatus): number | undefined {
  if (!state.total || !Number.isFinite(state.total) || state.total <= 0) return undefined;
  return Math.min(100, Math.max(0, Number.isFinite(state.downloaded) ? state.downloaded / state.total * 100 : 0));
}
export function updatePhaseText(state: UpdateStatus): string {
  switch (state.phase) {
    case 'checking': return t('正在检查新版本…');
    case 'up-to-date': return t('已检查，当前没有更新的版本。');
    case 'available': return state.available ? t('发现新版本 {version}。', { version: state.available.version }) : t('有新版本可用。');
    case 'downloading': return t('正在下载更新…');
    case 'ready': return state.available ? t('新版本 {version} 已下载并验证，等待安装。', { version: state.available.version }) : t('更新已下载并验证，等待安装。');
    case 'backing-up': return t('正在备份升级前的数据…');
    case 'installing': return t('正在安装更新，请稍候…');
    case 'installed': return t('新版已安装，重启星笺后生效。');
    case 'restart-required': return state.error ? errorText(state.error) : t('安装器未能启动，请重启星笺后再试。');
    case 'error': return state.error ? errorText(state.error) : t('更新未能完成，请重试。');
    default: return state.configured ? t('当前使用稳定版更新通道。') : t('此构建尚未配置更新服务。');
  }
}
export function fileSize(value: number): string {
  if (!Number.isFinite(value) || value < 0) return t('大小未知');
  if (value < 1024) return `${Math.round(value)} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let amount = value / 1024, unit = 0;
  while (amount >= 1024 && unit < units.length - 1) { amount /= 1024; unit++; }
  return `${formatNumber(amount, { maximumFractionDigits: 1 })} ${units[unit]}`;
}
