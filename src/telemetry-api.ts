export const telemetryDesktopAvailable = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
// The process heartbeat is emitted by the backend scheduler, never by a UI action.
export const usageEventNames = ['app_started', 'studio_opened', 'card_exported', 'backup_exported', 'backup_imported', 'update_check_completed', 'font_downloaded', 'update_download_completed', 'update_install_started'] as const;
export type UsageEvent = typeof usageEventNames[number];
export type TelemetryMode = 'off' | 'session' | 'installation';
export const telemetryModes: readonly TelemetryMode[] = ['off', 'session', 'installation'];
export type TelemetryState = {
  revision: number; mode: TelemetryMode; enabled: boolean; configured: boolean; endpointHost: string | null;
  queuedEvents: Array<{ name: UsageEvent; count: number }>;
  lastResult: 'never' | 'sent' | 'failed' | 'disabled' | 'unconfigured';
  // Local identity problem for `installation` mode. Never carries the identifier itself.
  identityError: string | null;
  identityPresent: boolean;
};
// Placeholder shown until the native state arrives. Controls stay disabled until then.
export function initialTelemetryState(): TelemetryState {
  return {
    revision: 0, mode: 'off', enabled: false, configured: false, endpointHost: null, queuedEvents: [],
    lastResult: 'never', identityError: null, identityPresent: false,
  };
}
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!telemetryDesktopAvailable) throw new Error('使用统计设置仅在桌面版可用。');
  return (await import('@tauri-apps/api/core')).invoke<T>(command, args);
}
export const getTelemetryState = () => invoke<TelemetryState>('get_telemetry_state');
// `off` | `session` | `installation`; only the user can pick per-installation reporting.
export const setTelemetryMode = (mode: TelemetryMode) => invoke<TelemetryState>('set_telemetry_mode', { mode });
export const resetTelemetryIdentity = () => invoke<TelemetryState>('reset_telemetry_identity');
export const flushTelemetry = () => invoke<TelemetryState>('flush_telemetry');
export const clearTelemetryQueue = () => invoke<TelemetryState>('clear_telemetry_queue');
export async function onTelemetryStateChanged(receive: (value: TelemetryState) => void): Promise<() => void> {
  if (!telemetryDesktopAvailable) return () => {};
  return (await import('@tauri-apps/api/event')).listen<TelemetryState>('starclip-telemetry-state', event => receive(event.payload));
}
// The backend owns the persisted preference and batching. Feature actions never wait for analytics.
export function recordStudioOpened(): void {
  if (telemetryDesktopAvailable) void invoke('record_usage_event', { name: 'studio_opened' }).catch(() => {});
}
