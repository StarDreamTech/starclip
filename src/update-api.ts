import packageInfo from '../package.json' with { type: 'json' };

export const updateDesktopAvailable = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
export const appVersion = packageInfo.version;
export type UpdatePhase = 'idle' | 'checking' | 'up-to-date' | 'available' | 'downloading' | 'ready' | 'backing-up' | 'installing' | 'installed' | 'restart-required' | 'error';
export type UpdateStatus = {
  revision: number;
  currentVersion: string;
  configured: boolean;
  phase: UpdatePhase;
  available?: { version: string; date?: string | null; notes?: string | null } | null;
  downloaded: number;
  total?: number | null;
  error?: string | null;
  backupId?: string | null;
};
export type UpdatePreferences = {
  revision: number;
  autoCheck: boolean;
  skippedVersion: string | null;
  lastCheckAt: number | null;
};
export function initialUpdatePreferences(): UpdatePreferences {
  return { revision: 0, autoCheck: true, skippedVersion: null, lastCheckAt: null };
}
export function getUpdatePreferences(): Promise<UpdatePreferences> {
  return updateDesktopAvailable ? invoke('get_update_preferences') : Promise.resolve(initialUpdatePreferences());
}
export function setAutoUpdateCheck(enabled: boolean): Promise<UpdatePreferences> {
  return invoke('set_auto_update_check', { enabled });
}
export function skipUpdateVersion(version: string): Promise<UpdatePreferences> {
  return invoke('skip_update_version', { version });
}
export async function onUpdatePreferencesChanged(callback: (value: UpdatePreferences) => void): Promise<() => void> {
  if (!updateDesktopAvailable) return () => {};
  return (await import('@tauri-apps/api/event')).listen<UpdatePreferences>('starclip-update-preferences', event => callback(event.payload));
}
export function initialUpdateStatus(): UpdateStatus {
  return { revision: 0, currentVersion: appVersion, configured: false, phase: 'idle', downloaded: 0 };
}
async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!updateDesktopAvailable) throw new Error('软件更新需要在星笺桌面版使用。');
  return (await import('@tauri-apps/api/core')).invoke<T>(command, args);
}

// Keep one listener through each mounted panel; delayed reads must not replace newer progress.
export function watchStarclipUpdates(onChange: (state: UpdateStatus) => void) {
  let disposed = false, revision = -1;
  let unsubscribe: (() => void) | undefined, subscribing: Promise<void> | undefined;
  function accept(state: UpdateStatus) {
    if (disposed || state.revision < revision) return;
    revision = state.revision;
    onChange(state);
  }
  async function subscribe() {
    if (disposed || unsubscribe) return;
    if (!subscribing) {
      subscribing = (async () => {
        const stop = await (await import('@tauri-apps/api/event')).listen<UpdateStatus>('starclip-update-status', event => accept(event.payload));
        if (disposed) stop(); else unsubscribe = stop;
      })().finally(() => { subscribing = undefined; });
    }
    await subscribing;
  }
  async function run(command: string, args?: Record<string, unknown>) {
    if (!updateDesktopAvailable || disposed) return;
    await subscribe();
    if (disposed) return;
    accept(await invoke<UpdateStatus>(command, args));
  }
  return {
    read: () => run('get_update_status'),
    check: () => run('check_starclip_update'),
    download: (expectedVersion: string) => run('download_starclip_update', { expectedVersion }),
    install: (expectedVersion: string) => run('install_starclip_update', { expectedVersion }),
    dispose() { disposed = true; unsubscribe?.(); unsubscribe = undefined; },
  };
}
export async function pendingDataDirectory(): Promise<string | null> {
  const value = await invoke<{ pendingPath?: string | null }>('get_data_location');
  return value.pendingPath ?? null;
}
export function restartAfterUpdate(): Promise<void> { return invoke('restart_app'); }
