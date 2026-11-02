//! Bounded optional counters. Session identities are ephemeral; installation identity is explicit.
//! Wire protocol 4 is independent of SQLite schema and never contains clipboard data.
use crate::store::Result;
use crate::telemetry_device::DeviceInfo;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const CACHE_TTL: u64 = 72 * 60 * 60;
const MAX_CACHE_BYTES: u64 = 2048;
const RETRY_DELAYS: [u64; 4] = [60, 300, 900, 3600];
const MAX_INSTALL_BYTES: u64 = 512;
const DAY: u64 = 24 * 60 * 60;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

const EVENTS: [&str; 10] = [
    "app_started",
    "studio_opened",
    "card_exported",
    "backup_exported",
    "backup_imported",
    "update_check_completed",
    "update_download_completed",
    "update_install_started",
    "font_downloaded",
    "app_active",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryMode {
    Off,
    Session,
    Installation,
}
impl TelemetryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Session => "session",
            Self::Installation => "installation",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "session" => Some(Self::Session),
            "installation" => Some(Self::Installation),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetryPreferences {
    pub revision: u64,
    pub mode: TelemetryMode,
}
impl Default for TelemetryPreferences {
    fn default() -> Self {
        Self {
            revision: 0,
            mode: TelemetryMode::Session,
        }
    }
}
impl<'de> Deserialize<'de> for TelemetryPreferences {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            revision: u64,
            #[serde(default)]
            mode: Option<String>,
            #[serde(default)]
            enabled: Option<bool>,
        }
        let raw = Raw::deserialize(deserializer)?;
        // Both fields would be ambiguous; preserve explicit opt-out rather than guess.
        let mode = match (raw.mode, raw.enabled) {
            (Some(value), None) => TelemetryMode::parse(&value)
                .ok_or_else(|| serde::de::Error::custom("统计模式无效"))?,
            (None, Some(true)) => TelemetryMode::Session,
            (None, Some(false)) => TelemetryMode::Off,
            _ => return Err(serde::de::Error::custom("统计偏好格式无效")),
        };
        Ok(Self {
            revision: raw.revision,
            mode,
        })
    }
}
impl Serialize for TelemetryPreferences {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("TelemetryPreferences", 2)?;
        state.serialize_field("revision", &self.revision)?;
        state.serialize_field("mode", self.mode.as_str())?;
        state.end()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventCount {
    name: &'static str,
    count: u8,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryState {
    pub revision: u64,
    pub enabled: bool,
    pub mode: &'static str,
    pub identity_error: Option<&'static str>,
    pub cache_error: Option<&'static str>,
    pub identity_present: bool,
    pub configured: bool,
    pub endpoint_host: Option<String>,
    pub queued_events: Vec<EventCount>,
    pub last_result: &'static str,
    /// Only a short prefix so the user can see a reset take effect.
    pub install_hint: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    version: u8,
    session_id: String,
    app_version: &'static str,
    platform: &'static str,
    device: DeviceInfo,
    identity_mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    install_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    first_seen_day: Option<String>,
    events: Vec<EventCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallId {
    version: u8,
    install_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_at: Option<u64>,
}

fn valid_hex_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// UTC calendar date for GA cohorts; day precision only.
fn utc_day(seconds: u64) -> String {
    // Howard Hinnant's civil_from_days, valid for all dates after 1970.
    let z = (seconds / DAY) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

pub struct Batch {
    endpoint: Url,
    envelope: Envelope,
    generation: u64,
    created_at: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CachedCounts {
    version: u8,
    app_version: String,
    created_at: u64,
    mode: String,
    identity_fingerprint: String,
    counts: BTreeMap<String, u8>,
}

struct InFlight {
    session_id: String,
    generation: u64,
    created_at: u64,
    counts: [u8; EVENTS.len()],
}

pub struct Telemetry {
    endpoint: Option<Url>,
    session_id: Option<String>,
    mode: TelemetryMode,
    preferences_known: bool,
    cache_error: Option<&'static str>,
    identity_error: Option<&'static str>,
    revoke_pending: bool,
    heartbeat: bool,
    next_heartbeat_at: u64,
    revision: u64,
    counts: [u8; EVENTS.len()],
    in_flight: Option<InFlight>,
    last_result: &'static str,
    device: DeviceInfo,
    cache_path: Option<PathBuf>,
    created_at: Option<u64>,
    generation: u64,
    failures: usize,
    next_attempt_at: u64,
    dirty: bool,
    install: Option<InstallId>,
    install_path: Option<PathBuf>,
    active_day: Option<u64>,
}

fn endpoint(value: Option<&str>) -> Option<Url> {
    let url = Url::parse(value?).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/collect"
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

/// Atomic owner-only write via a sibling temporary file.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other("identity/cache symlink refused"));
    }
    let temporary = path.with_extension("tmp");
    let written = (|| {
        let _ = fs::remove_file(&temporary);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    let _ = fs::remove_file(temporary);
    written
}

fn session_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成临时统计会话，请重试。".to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Telemetry {
    pub fn new(preferences: Result<TelemetryPreferences>) -> Self {
        let mut telemetry =
            Self::with_endpoint(preferences, option_env!("STARCLIP_TELEMETRY_ENDPOINT"));
        if telemetry.enabled() {
            telemetry.load_device();
        }
        telemetry
    }

    fn with_endpoint(preferences: Result<TelemetryPreferences>, value: Option<&str>) -> Self {
        // Only the missing-key path may choose the product default. Corrupt or
        // unreadable preferences must never accidentally turn collection on.
        let preferences_known = preferences.is_ok();
        let preferences = preferences.unwrap_or(TelemetryPreferences {
            revision: 0,
            mode: TelemetryMode::Off,
        });
        let endpoint = endpoint(value);
        let session_id = session_id().ok();
        let mode = preferences.mode;
        Self {
            last_result: if endpoint.is_some() {
                "never"
            } else {
                "unconfigured"
            },
            endpoint,
            session_id,
            mode,
            preferences_known,
            cache_error: None,
            identity_error: if preferences_known {
                None
            } else {
                Some("统计偏好未能读取，已停止统计；原标识与缓存已保留，请重新选择统计模式。")
            },
            revoke_pending: false,
            heartbeat: false,
            next_heartbeat_at: 0,
            revision: preferences.revision,
            counts: [0; EVENTS.len()],
            in_flight: None,
            device: DeviceInfo {
                architecture: "unknown".into(),
                os_major: "unknown".into(),
                model: "unknown".into(),
            },
            cache_path: None,
            created_at: None,
            generation: 0,
            failures: 0,
            next_attempt_at: 0,
            dirty: false,
            install: None,
            install_path: None,
            active_day: None,
        }
    }

    /// Sidecars are independent of the clipboard library. Unconfigured previews never touch them.
    pub fn with_install(mut self, path: PathBuf) -> Self {
        self.install_path = Some(path);
        if self.endpoint.is_some() && self.preferences_known {
            if self.mode == TelemetryMode::Installation {
                let _ = self.ensure_install();
            } else {
                let _ = self.forget_install();
            }
        }
        if self.enabled() {
            self.load_device();
        }
        self
    }

    fn identity_paths(&self) -> Option<[PathBuf; 2]> {
        let current = self.install_path.as_ref()?;
        Some([
            current.clone(),
            current.with_file_name("usage-identity.json"),
        ])
    }

    fn read_install(path: &Path, legacy: bool) -> Result<Option<InstallId>> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("统计标识未能读取。".into()),
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_INSTALL_BYTES
        {
            return Err("统计标识格式无效。".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(path)
            .and_then(|file| file.take(MAX_INSTALL_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|_| "统计标识未能读取。")?;
        if bytes.len() as u64 > MAX_INSTALL_BYTES {
            return Err("统计标识格式无效。".into());
        }
        let value = if legacy {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Legacy {
                version: u8,
                installation_id: String,
            }
            let saved: Legacy = serde_json::from_slice(&bytes).map_err(|_| "统计标识格式无效。")?;
            if saved.version != 1 {
                return Err("统计标识版本无效。".into());
            }
            InstallId {
                version: 2,
                install_id: saved.installation_id,
                created_at: None,
            }
        } else {
            let saved: InstallId =
                serde_json::from_slice(&bytes).map_err(|_| "统计标识格式无效。")?;
            if ![1, 2].contains(&saved.version)
                || (saved.version == 1 && saved.created_at.is_none())
            {
                return Err("统计标识版本无效。".into());
            }
            saved
        };
        if !valid_hex_id(&value.install_id)
            || value
                .created_at
                .is_some_and(|time| time > now() || utc_day(time) < "2026-01-01".to_owned())
        {
            return Err("统计标识格式无效。".into());
        }
        Ok(Some(value))
    }

    fn ensure_install(&mut self) -> Result<()> {
        if self.mode != TelemetryMode::Installation || self.endpoint.is_none() {
            return Ok(());
        }
        if self.revoke_pending {
            self.forget_install()?;
        }
        let Some([current, legacy]) = self.identity_paths() else {
            self.identity_error = Some("未配置统计标识位置，已停止按安装统计。");
            return Err(self.identity_error.unwrap().into());
        };
        let loaded = (|| -> Result<InstallId> {
            let current_saved = Self::read_install(&current, false)?;
            let legacy_saved = Self::read_install(&legacy, true)?;
            if let (Some(current), Some(legacy)) = (&current_saved, &legacy_saved) {
                if current.install_id != legacy.install_id {
                    return Err("两套统计标识不一致，未选择或删除任何编号。".into());
                }
            }
            let already_saved = current_saved.is_some();
            let saved = match current_saved.or(legacy_saved) {
                Some(saved) => saved,
                None => InstallId {
                    version: 2,
                    install_id: session_id()?,
                    created_at: Some(now()),
                },
            };
            #[cfg(unix)]
            if already_saved {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&current, fs::Permissions::from_mode(0o600))
                    .map_err(|_| "统计标识权限未能收紧。")?;
            }
            if !already_saved {
                write_private(
                    &current,
                    &serde_json::to_vec(&saved).map_err(|_| "统计标识未能保存。")?,
                )
                .map_err(|_| "统计标识未能保存。")?;
            }
            // Only remove the old representation after the selected ID is durable.
            match fs::remove_file(&legacy) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("旧统计标识未能删除。".into()),
            }
            Ok(saved)
        })();
        match loaded {
            Ok(saved) => {
                self.install = Some(saved);
                self.identity_error = None;
                Ok(())
            }
            Err(error) => {
                self.install = None;
                self.identity_error = Some(
                    if error == "两套统计标识不一致，未选择或删除任何编号。" {
                        "两套统计标识不一致，已停止按安装统计；两个文件均已保留，可显式重置编号。"
                    } else {
                        "统计标识未能读取、保存或清理，已停止按安装统计；请修复目录后重试。"
                    },
                );
                Err(self.identity_error.unwrap().into())
            }
        }
    }

    fn forget_install(&mut self) -> Result<()> {
        self.install = None;
        let Some(paths) = self.identity_paths() else {
            return Ok(());
        };
        let mut failed = false;
        for path in paths {
            for file in [path.clone(), path.with_extension("tmp")] {
                if fs::remove_file(file)
                    .is_err_and(|error| error.kind() != std::io::ErrorKind::NotFound)
                {
                    failed = true;
                }
            }
        }
        self.revoke_pending = failed;
        if failed {
            self.identity_error = Some("统计标识未能删除；请检查目录写入权限后重试。");
            Err(self.identity_error.unwrap().into())
        } else {
            self.identity_error = None;
            Ok(())
        }
    }

    /// A failed revocation must succeed before installation mode can be persisted again.
    pub fn retry_identity(&mut self) -> Result<()> {
        if self.endpoint.is_none() {
            return Err("此版本尚未配置统计接收服务，无法开启上报。".into());
        }
        self.clear();
        let result = if self.mode == TelemetryMode::Installation {
            self.ensure_install()
        } else {
            self.forget_install()
        };
        self.bump();
        result
    }

    pub fn reset_install(&mut self) -> Result<()> {
        if self.endpoint.is_none() {
            return Err("此版本尚未配置统计接收服务，无法开启上报。".into());
        }
        if self.mode != TelemetryMode::Installation {
            return Err("只有按安装统计时才需要重置标识。".into());
        }
        self.clear();
        self.active_day = None;
        self.forget_install()?;
        self.ensure_install()?;
        self.tick();
        self.bump();
        Ok(())
    }

    /// At most one `app_active` per UTC day per run, so long-running tray sessions count as daily users.
    fn mark_active(&mut self, at: u64) {
        let day = at / DAY;
        if self.enabled()
            && self.mode == TelemetryMode::Installation
            && self.active_day != Some(day)
        {
            self.active_day = Some(day);
            let _ = self.record("app_active");
        }
    }

    fn identity_fingerprint(&self) -> Option<String> {
        let value = match self.mode {
            TelemetryMode::Off => return None,
            TelemetryMode::Session => self.session_id.as_deref()?,
            TelemetryMode::Installation => self.install.as_ref()?.install_id.as_str(),
        };
        Some(format!("{:x}", Sha256::digest(value.as_bytes())))
    }

    /// v3 caches name each event and bind counts to the exact identity without storing its ID.
    /// Legacy arrays cannot establish identity ownership; discard rather than reattribute them.
    pub fn with_cache(mut self, path: PathBuf) -> Self {
        if self.endpoint.is_none() {
            return self;
        }
        self.cache_path = Some(path.clone());
        if !self.preferences_known {
            return self;
        }
        let restored = (|| -> Option<CachedCounts> {
            if !self.enabled() {
                return None;
            }
            let meta = fs::symlink_metadata(&path).ok()?;
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_CACHE_BYTES {
                return None;
            }
            let mut bytes = Vec::new();
            fs::File::open(&path)
                .ok()?
                .take(MAX_CACHE_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > MAX_CACHE_BYTES {
                return None;
            }
            let saved: CachedCounts = serde_json::from_slice(&bytes).ok()?;
            let age = now().checked_sub(saved.created_at)?;
            (saved.version == 3
                && saved.app_version == env!("CARGO_PKG_VERSION")
                && saved.mode == self.mode.as_str()
                && Some(saved.identity_fingerprint.as_str())
                    == self.identity_fingerprint().as_deref()
                && age < CACHE_TTL
                && saved.counts.len() <= EVENTS.len()
                && saved.counts.iter().all(|(key, count)| {
                    EVENTS.contains(&key.as_str())
                        && *count <= 100
                        && (self.mode == TelemetryMode::Installation || key != "app_active")
                }))
            .then_some(saved)
        })();
        if let Some(saved) = restored {
            for (index, event) in EVENTS.iter().enumerate() {
                self.counts[index] = *saved.counts.get(*event).unwrap_or(&0);
            }
            self.created_at = Some(saved.created_at);
        } else {
            self.dirty = true;
            self.checkpoint();
        }
        let _ = fs::remove_file(path.with_extension("tmp"));
        self
    }

    fn expire(&mut self, at: u64) {
        if self
            .created_at
            .is_some_and(|start| at < start || at - start >= CACHE_TTL)
        {
            self.counts = [0; EVENTS.len()];
            self.created_at = None;
            self.dirty = true;
            self.bump();
        }
    }

    /// Called by the existing minute timer and at normal exit, never by a UI network wait.
    /// The minute timer: count today's activity, then persist.
    pub fn tick(&mut self) {
        let at = now();
        self.mark_active(at);
        if self.enabled() && self.mode == TelemetryMode::Installation {
            if self.next_heartbeat_at == 0 || at >= self.next_heartbeat_at {
                self.heartbeat = true;
                self.next_heartbeat_at = at.saturating_add(3600);
            }
        } else {
            self.heartbeat = false;
            self.next_heartbeat_at = 0;
        }
        self.checkpoint();
    }

    pub fn checkpoint(&mut self) {
        if self.endpoint.is_none() || !self.preferences_known {
            return;
        }
        self.expire(now());
        if !self.dirty {
            return;
        }
        let Some(path) = &self.cache_path else {
            return;
        };
        let mut counts = self.counts;
        let mut created_at = self.created_at;
        if let Some(flight) = &self.in_flight {
            if flight.generation == self.generation && self.enabled() {
                for (count, previous) in counts.iter_mut().zip(flight.counts) {
                    *count = count.saturating_add(previous).min(100);
                }
                created_at =
                    Some(created_at.map_or(flight.created_at, |at| at.min(flight.created_at)));
            }
        }
        if !self.enabled() || !counts.iter().any(|&n| n > 0) {
            let mut removed = true;
            for file in [path.clone(), path.with_extension("tmp")] {
                if fs::remove_file(file)
                    .is_err_and(|error| error.kind() != std::io::ErrorKind::NotFound)
                {
                    removed = false;
                }
            }
            self.dirty = !removed;
            let next_error = if removed {
                None
            } else {
                Some("本次待发统计已清空，但本地缓存未能删除；请恢复目录写入权限后重试，重启可能恢复旧计数。")
            };
            if self.cache_error != next_error {
                self.cache_error = next_error;
                self.bump();
            }
            return;
        }
        let saved = CachedCounts {
            version: 3,
            app_version: env!("CARGO_PKG_VERSION").into(),
            created_at: created_at.unwrap_or_else(now),
            mode: self.mode.as_str().into(),
            identity_fingerprint: self.identity_fingerprint().unwrap_or_default(),
            counts: EVENTS
                .iter()
                .zip(counts)
                .filter(|(_, count)| *count > 0)
                .map(|(event, count)| ((*event).into(), count))
                .collect(),
        };
        let written = serde_json::to_vec(&saved)
            .map_err(std::io::Error::other)
            .and_then(|bytes| write_private(path, &bytes));
        if written.is_ok() {
            self.dirty = false;
            if self.cache_error.take().is_some() {
                self.bump();
            }
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn state(&self) -> TelemetryState {
        TelemetryState {
            revision: self.revision,
            enabled: self.enabled(),
            mode: self.mode.as_str(),
            identity_error: self.identity_error,
            cache_error: self.cache_error,
            identity_present: self.install.is_some(),
            configured: self.endpoint.is_some(),
            endpoint_host: self
                .endpoint
                .as_ref()
                .and_then(|url| url.host_str().map(str::to_owned)),
            queued_events: EVENTS
                .iter()
                .zip(self.counts)
                .filter_map(|(&name, count)| (count > 0).then_some(EventCount { name, count }))
                .collect(),
            last_result: self.last_result,
            install_hint: self
                .install
                .as_ref()
                .filter(|_| self.enabled())
                .map(|install| install.install_id[..6].to_owned()),
        }
    }

    fn load_device(&mut self) {
        if self.device.model != "unknown" {
            return;
        }
        #[cfg(not(test))]
        {
            self.device = DeviceInfo::detect();
        }
        #[cfg(test)]
        {
            self.device = DeviceInfo {
                architecture: "unknown".into(),
                os_major: "unknown".into(),
                model: match std::env::consts::OS {
                    "macos" => "Mac",
                    "windows" => "WindowsPC",
                    _ => "LinuxPC",
                }
                .into(),
            };
        }
    }

    pub fn enabled(&self) -> bool {
        self.preferences_known
            && self.endpoint.is_some()
            && self.session_id.is_some()
            && self.mode != TelemetryMode::Off
            && (self.mode != TelemetryMode::Installation
                || (self.install.is_some() && self.identity_error.is_none()))
    }

    pub fn validate_enable(&mut self) -> Result<()> {
        self.validate_mode(TelemetryMode::Session)
    }
    pub fn validate_mode(&mut self, mode: TelemetryMode) -> Result<()> {
        if self.endpoint.is_none() {
            return Err("此版本尚未配置统计接收服务，无法开启上报。".into());
        }
        if self.session_id.is_none() {
            self.session_id = Some(session_id()?);
        }
        if mode == TelemetryMode::Installation && self.revoke_pending {
            self.forget_install()?;
        }
        self.load_device();
        Ok(())
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.set_mode(if enabled {
            TelemetryMode::Session
        } else {
            TelemetryMode::Off
        });
    }
    pub fn set_mode(&mut self, mode: TelemetryMode) {
        // Only an explicit choice can turn an unreadable preference into a known one.
        self.preferences_known = true;
        self.clear();
        self.mode = mode;
        self.session_id = session_id().ok();
        self.active_day = None;
        if self.endpoint.is_some() {
            if mode == TelemetryMode::Installation {
                let _ = self.ensure_install();
            } else {
                let _ = self.forget_install();
            }
        }
        self.last_result = if self.endpoint.is_none() {
            "unconfigured"
        } else if self.enabled() {
            "never"
        } else {
            "disabled"
        };
        self.bump();
    }

    pub fn record(&mut self, name: &str) -> Result<()> {
        if name == "app_active" && self.mode != TelemetryMode::Installation {
            return Err("此统计事件仅在本机固定编号模式下使用。".into());
        }
        let index = EVENTS
            .iter()
            .position(|event| *event == name)
            .ok_or("不支持此统计事件。")?;
        self.expire(now());
        if self.enabled() && self.counts[index] < 100 {
            self.counts[index] += 1;
            self.created_at.get_or_insert_with(now);
            self.dirty = true;
            self.bump();
        }
        Ok(())
    }

    pub fn record_frontend(&mut self, name: &str) -> Result<()> {
        if name != "studio_opened" {
            return Err("此事件只能由应用完成操作后记录。".into());
        }
        self.record(name)
    }

    pub fn clear(&mut self) {
        self.counts = [0; EVENTS.len()];
        self.created_at = None;
        self.heartbeat = false;
        self.next_heartbeat_at = 0;
        self.generation = self.generation.wrapping_add(1);
        self.in_flight = None;
        self.failures = 0;
        self.next_attempt_at = 0;
        self.dirty = true;
        self.checkpoint();
        self.bump();
    }

    pub fn clear_queue(&mut self) -> Result<()> {
        let scheduled = self.next_heartbeat_at;
        self.clear();
        self.next_heartbeat_at = scheduled;
        if self.dirty && self.cache_path.is_some() {
            Err("本次待发统计已清空，但本地缓存未能删除；请恢复目录写入权限后重试，重启可能恢复旧计数。".into())
        } else {
            Ok(())
        }
    }

    pub fn ready(&self) -> bool {
        self.enabled()
            && self.in_flight.is_none()
            && now() >= self.next_attempt_at
            && (self.counts.iter().any(|count| *count > 0) || self.heartbeat)
    }

    // Taking a batch is the request start boundary. Revocation clears everything
    // not yet started; an in-flight HTTP request cannot be recalled.
    pub fn take_batch(&mut self) -> Option<Batch> {
        self.checkpoint();
        if !self.ready() {
            return None;
        }
        let session_id = self.session_id.clone()?;
        let install = if self.mode == TelemetryMode::Installation {
            self.install.clone()
        } else {
            None
        };
        let mut events = self.state().queued_events;
        if self.heartbeat {
            events.push(EventCount {
                name: "app_running",
                count: 1,
            });
        }
        self.heartbeat = false;
        let batch = Batch {
            endpoint: self.endpoint.clone()?,
            envelope: Envelope {
                version: 4,
                session_id: session_id.clone(),
                app_version: env!("CARGO_PKG_VERSION"),
                platform: std::env::consts::OS,
                device: self.device.clone(),
                identity_mode: self.mode.as_str(),
                install_id: install.as_ref().map(|saved| saved.install_id.clone()),
                first_seen_day: install.and_then(|saved| saved.created_at).map(utc_day),
                events,
            },
            generation: self.generation,
            created_at: self.created_at.unwrap_or_else(now),
        };
        self.in_flight = Some(InFlight {
            session_id,
            generation: self.generation,
            created_at: batch.created_at,
            counts: self.counts,
        });
        self.counts = [0; EVENTS.len()];
        self.created_at = None;
        self.bump();
        Some(batch)
    }

    pub fn finish(&mut self, batch: &Batch, success: bool) {
        if !self.in_flight.as_ref().is_some_and(|f| {
            f.session_id == batch.envelope.session_id && f.generation == batch.generation
        }) {
            return;
        }
        let flight = self.in_flight.take().unwrap();
        if self.enabled()
            && self.generation == batch.generation
            && self.session_id.as_ref() == Some(&batch.envelope.session_id)
        {
            self.last_result = if success { "sent" } else { "failed" };
            if success {
                self.failures = 0;
                self.next_attempt_at = 0;
            } else {
                let at = now();
                if at >= flight.created_at && at - flight.created_at < CACHE_TTL {
                    for (count, previous) in self.counts.iter_mut().zip(flight.counts) {
                        *count = count.saturating_add(previous).min(100);
                    }
                    self.created_at = Some(
                        self.created_at
                            .map_or(flight.created_at, |time| time.min(flight.created_at)),
                    );
                }
                self.next_attempt_at =
                    at.saturating_add(RETRY_DELAYS[self.failures.min(RETRY_DELAYS.len() - 1)]);
                self.failures = (self.failures + 1).min(RETRY_DELAYS.len());
            }
            self.dirty = true;
            self.checkpoint();
        }
        self.bump();
    }
}

pub fn send(batch: &Batch) -> bool {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let Ok(client) = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return false;
    };
    let Ok(body) = serde_json::to_vec(&batch.envelope) else {
        return false;
    };
    client
        .post(batch.endpoint.clone())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .is_ok_and(|response| response.status() == reqwest::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("starclip-telemetry-v4-{}", session_id().unwrap()));
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn id(&self) -> PathBuf {
            self.0.join("usage-install.json")
        }
        fn old(&self) -> PathBuf {
            self.0.join("usage-identity.json")
        }
        fn cache(&self) -> PathBuf {
            self.0.join("usage-counts.json")
        }
        fn installation(&self) -> Telemetry {
            configured(TelemetryMode::Installation)
                .with_install(self.id())
                .with_cache(self.cache())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn configured(mode: TelemetryMode) -> Telemetry {
        Telemetry::with_endpoint(
            Ok(TelemetryPreferences { revision: 3, mode }),
            Some("https://stats.example/collect"),
        )
    }
    fn payload(batch: &Batch) -> serde_json::Value {
        serde_json::to_value(&batch.envelope).unwrap()
    }

    #[test]
    fn preferences_preserve_all_modes_and_legacy_boolean_meaning() {
        assert_eq!(TelemetryPreferences::default().mode, TelemetryMode::Session);
        for (json, expected) in [
            (r#"{"revision":7,"enabled":false}"#, TelemetryMode::Off),
            (r#"{"revision":7,"enabled":true}"#, TelemetryMode::Session),
            (r#"{"revision":7,"mode":"off"}"#, TelemetryMode::Off),
            (r#"{"revision":7,"mode":"session"}"#, TelemetryMode::Session),
            (
                r#"{"revision":7,"mode":"installation"}"#,
                TelemetryMode::Installation,
            ),
        ] {
            let saved: TelemetryPreferences = serde_json::from_str(json).unwrap();
            assert_eq!(saved.mode, expected);
            assert_eq!(saved.revision, 7);
            assert_eq!(
                serde_json::to_value(saved).unwrap(),
                serde_json::json!({"revision":7,"mode":expected.as_str()})
            );
        }
        for invalid in [
            r#"{"revision":7}"#,
            r#"{"enabled":true}"#,
            r#"{"revision":7,"mode":"unknown"}"#,
            r#"{"revision":7,"mode":"session","enabled":true}"#,
        ] {
            assert!(serde_json::from_str::<TelemetryPreferences>(invalid).is_err());
        }
    }

    #[test]
    fn unconfigured_previews_and_read_errors_never_touch_identity_or_collect() {
        let f = Fixture::new();
        fs::write(f.id(), b"original untouched fixture").unwrap();
        for mode in [
            TelemetryMode::Off,
            TelemetryMode::Session,
            TelemetryMode::Installation,
        ] {
            for endpoint in [
                None,
                Some("http://stats.example/collect"),
                Some("https://stats.example/collect?secret=x"),
            ] {
                let mut t = Telemetry::with_endpoint(
                    Ok(TelemetryPreferences { revision: 0, mode }),
                    endpoint,
                )
                .with_install(f.id())
                .with_cache(f.cache());
                t.record("app_started").unwrap();
                t.tick();
                assert!(!t.enabled());
                assert!(t.take_batch().is_none());
                assert!(t.reset_install().is_err());
                assert!(t.retry_identity().is_err());
                assert_eq!(fs::read(f.id()).unwrap(), b"original untouched fixture");
            }
        }
        let mut t = Telemetry::with_endpoint(
            Err("fixed read failure".into()),
            Some("https://stats.example/collect"),
        );
        t.record("app_started").unwrap();
        assert!(!t.enabled());
        assert_eq!(t.mode, TelemetryMode::Off);
    }

    #[test]
    fn session_protocol_never_contains_persistent_identity_or_running_events() {
        let mut t = configured(TelemetryMode::Session);
        t.record("studio_opened").unwrap();
        t.tick();
        let batch = t.take_batch().unwrap();
        let value = payload(&batch);
        assert_eq!(value["version"], 4);
        assert_eq!(value["identityMode"], "session");
        assert_eq!(value.as_object().unwrap().len(), 7);
        assert!(value.get("installId").is_none() && value.get("firstSeenDay").is_none());
        assert_eq!(
            value["events"],
            serde_json::json!([{"name":"studio_opened","count":1}])
        );
        assert!(t.record_frontend("app_active").is_err());
        assert!(t.record("app_running").is_err());
    }

    #[test]
    fn main_identity_retains_id_and_first_seen_while_wip_identity_has_no_invented_date() {
        let id = "0123456789abcdef0123456789abcdef";
        for legacy in [false, true] {
            let f = Fixture::new();
            let original = if legacy {
                serde_json::json!({"version":1,"installationId":id})
            } else {
                serde_json::json!({"version":1,"installId":id,"createdAt":1793404800_u64})
            };
            fs::write(
                if legacy { f.old() } else { f.id() },
                serde_json::to_vec(&original).unwrap(),
            )
            .unwrap();
            let mut t = f.installation();
            assert!(t.enabled());
            t.tick();
            let batch = t.take_batch().unwrap();
            let value = payload(&batch);
            assert_eq!(value["installId"], id);
            assert_eq!(value["identityMode"], "installation");
            if legacy {
                assert!(value.get("firstSeenDay").is_none());
            } else {
                assert_eq!(value["firstSeenDay"], "2026-10-31");
            }
            assert!(!f.old().exists());
            assert_eq!(f.installation().install.unwrap().install_id, id);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(f.id()).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }

    #[test]
    fn invalid_identity_is_fail_closed_and_new_identity_writes_must_succeed() {
        let f = Fixture::new();
        fs::write(f.id(), b"broken original fixture").unwrap();
        let mut t = f.installation();
        assert!(!t.enabled());
        assert!(t.state().identity_error.is_some());
        assert_eq!(fs::read(f.id()).unwrap(), b"broken original fixture");
        t.record("app_started").unwrap();
        assert!(t.take_batch().is_none());
        fs::remove_file(f.id()).unwrap();
        t.retry_identity().unwrap();
        assert!(t.enabled());
        let path = f.0.join("not-created/usage-install.json");
        let mut failed = configured(TelemetryMode::Installation).with_install(path.clone());
        assert!(!failed.enabled());
        assert!(failed.state().identity_error.is_some());
        fs::create_dir(path.parent().unwrap()).unwrap();
        failed.retry_identity().unwrap();
        assert!(failed.enabled());
    }

    #[cfg(unix)]
    #[test]
    fn identity_symlinks_are_never_followed_or_overwritten() {
        let f = Fixture::new();
        let target = f.0.join("original.txt");
        fs::write(&target, b"fixed fixture").unwrap();
        std::os::unix::fs::symlink(&target, f.id()).unwrap();
        let mut t = f.installation();
        assert!(!t.enabled());
        assert!(t.state().identity_error.is_some());
        t.set_mode(TelemetryMode::Off);
        assert_eq!(fs::read(&target).unwrap(), b"fixed fixture");
        assert!(!f.id().exists());
    }

    #[test]
    fn revocation_retries_both_file_names_without_hiding_deletion_errors() {
        let f = Fixture::new();
        let mut t = f.installation();
        fs::create_dir(f.old()).unwrap();
        t.set_mode(TelemetryMode::Off);
        assert!(!t.enabled());
        assert!(!f.id().exists());
        assert!(t.state().identity_error.is_some());
        t.set_mode(TelemetryMode::Off);
        assert!(t.state().identity_error.is_some());
        assert!(t.validate_mode(TelemetryMode::Installation).is_err());
        fs::remove_dir(f.old()).unwrap();
        t.retry_identity().unwrap();
        assert!(t.state().identity_error.is_none());
        t.set_mode(TelemetryMode::Installation);
        assert!(t.enabled());
        t.set_mode(TelemetryMode::Session);
        assert!(!f.id().exists());
        assert!(t.enabled());
    }

    #[test]
    fn resetting_or_switching_identity_drops_counts_in_flight_cache_and_stale_callbacks() {
        let f = Fixture::new();
        let mut t = f.installation();
        let previous = t.install.as_ref().unwrap().install_id.clone();
        t.record("card_exported").unwrap();
        let old = t.take_batch().unwrap();
        t.record("studio_opened").unwrap();
        t.reset_install().unwrap();
        assert_ne!(t.install.as_ref().unwrap().install_id, previous);
        let new = t.take_batch().unwrap();
        let value = payload(&new);
        assert!(!value["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "card_exported" || e["name"] == "studio_opened"));
        t.finish(&old, false);
        assert!(t.state().queued_events.is_empty());
        t.finish(&new, true);
        t.record("backup_exported").unwrap();
        t.set_mode(TelemetryMode::Off);
        assert!(t.take_batch().is_none());
        assert!(!f.id().exists());
        assert!(!f.cache().exists());
        t.finish(&new, false);
        assert!(t.state().queued_events.is_empty());
        assert!(!f.cache().exists());
    }

    #[test]
    fn a_failed_reset_stops_installation_sending_and_preserves_a_visible_retry() {
        let f = Fixture::new();
        let mut t = f.installation();
        t.record("card_exported").unwrap();
        let old = t.take_batch().unwrap();
        fs::create_dir(f.old()).unwrap();
        assert!(t.reset_install().is_err());
        assert!(!t.enabled());
        assert!(t.state().identity_error.is_some());
        assert!(t.state().queued_events.is_empty());
        t.finish(&old, false);
        assert!(t.state().queued_events.is_empty());
        fs::remove_dir(f.old()).unwrap();
        t.retry_identity().unwrap();
        assert!(t.enabled());
    }

    #[test]
    fn named_cache_restores_only_the_same_installation_and_contains_no_id_or_heartbeat() {
        let f = Fixture::new();
        let mut t = f.installation();
        let id = t.install.as_ref().unwrap().install_id.clone();
        t.record("studio_opened").unwrap();
        t.tick();
        t.checkpoint();
        let bytes = fs::read(f.cache()).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved["version"], 3);
        assert_eq!(saved["counts"]["studio_opened"], 1);
        assert!(saved["counts"].get("app_running").is_none());
        assert!(!String::from_utf8(bytes).unwrap().contains(&id));
        let restored = f.installation();
        assert_eq!(restored.counts[1], 1);
        let old = fs::read(f.cache()).unwrap();
        let mut next = f.installation();
        next.reset_install().unwrap();
        fs::write(f.cache(), old).unwrap();
        assert!(f.installation().state().queued_events.is_empty());
    }

    #[test]
    fn legacy_arrays_foreign_modes_and_unknown_named_events_are_discarded_not_reattributed() {
        let f = Fixture::new();
        let t = f.installation();
        let fingerprint = t.identity_fingerprint().unwrap();
        for count in [9, 10] {
            fs::write(f.cache(),serde_json::to_vec(&serde_json::json!({"version":2,"appVersion":env!("CARGO_PKG_VERSION"),"createdAt":now(),"counts":vec![1;count]})).unwrap()).unwrap();
            assert!(f.installation().state().queued_events.is_empty());
            assert!(!f.cache().exists());
        }
        for counts in [
            serde_json::json!({"app_running":1}),
            serde_json::json!({"card_exported":101}),
            serde_json::json!({"private_unknown":1}),
        ] {
            fs::write(f.cache(),serde_json::to_vec(&serde_json::json!({"version":3,"appVersion":env!("CARGO_PKG_VERSION"),"createdAt":now(),"mode":"installation","identityFingerprint":fingerprint,"counts":counts})).unwrap()).unwrap();
            assert!(f.installation().state().queued_events.is_empty());
        }
        let mut session = configured(TelemetryMode::Session)
            .with_install(f.id())
            .with_cache(f.cache());
        session.record("app_started").unwrap();
        session.checkpoint();
        assert!(configured(TelemetryMode::Session)
            .with_install(f.id())
            .with_cache(f.cache())
            .state()
            .queued_events
            .is_empty());
    }

    #[test]
    fn hourly_heartbeat_is_installation_only_never_replayed_and_catches_up_once() {
        let f = Fixture::new();
        let mut t = f.installation();
        t.tick();
        let batch = t.take_batch().unwrap();
        assert_eq!(
            batch
                .envelope
                .events
                .iter()
                .filter(|e| e.name == "app_running")
                .count(),
            1
        );
        t.finish(&batch, false);
        t.next_attempt_at = 0;
        let retried = t.take_batch().unwrap();
        assert!(retried
            .envelope
            .events
            .iter()
            .all(|e| e.name != "app_running"));
        t.finish(&retried, true);
        t.tick();
        assert!(t.take_batch().is_none());
        t.next_heartbeat_at = now() - 3600 * 5;
        t.tick();
        let next = t.take_batch().unwrap();
        assert_eq!(next.envelope.events.len(), 1);
        assert_eq!(next.envelope.events[0].name, "app_running");
        t.finish(&next, true);
        t.tick();
        assert!(t.take_batch().is_none());
    }

    #[test]
    fn feature_counts_are_bounded_and_retry_with_a_fixed_backoff() {
        let mut t = configured(TelemetryMode::Session);
        assert!(t.record("clipboard_secret").is_err());
        assert!(t.record_frontend("card_exported").is_err());
        for _ in 0..200 {
            t.record("studio_opened").unwrap();
        }
        let batch = t.take_batch().unwrap();
        assert_eq!(batch.envelope.events[0].count, 100);
        t.finish(&batch, false);
        assert!(t.take_batch().is_none());
        assert!((now() + 59..=now() + 61).contains(&t.next_attempt_at));
        t.next_attempt_at = 0;
        let retry = t.take_batch().unwrap();
        t.finish(&retry, true);
        assert!(t.state().queued_events.is_empty());
        assert_eq!(t.failures, 0);
    }

    #[test]
    fn daily_activity_does_not_return_after_clearing_and_never_occurs_in_session() {
        let f = Fixture::new();
        let mut t = f.installation();
        t.tick();
        t.tick();
        assert_eq!(t.counts[9], 1);
        t.clear_queue().unwrap();
        t.tick();
        assert!(t.take_batch().is_none());
        t.mark_active(now() + DAY);
        assert_eq!(t.counts[9], 1);
        let mut session = configured(TelemetryMode::Session);
        session.tick();
        assert!(session.take_batch().is_none());
    }

    #[test]
    fn conflicting_identity_files_are_preserved_until_explicit_reset_and_matching_ids_merge() {
        let f = Fixture::new();
        let current = serde_json::json!({"version":1,"installId":"0123456789abcdef0123456789abcdef","createdAt":1793404800_u64});
        let legacy =
            serde_json::json!({"version":1,"installationId":"fedcba9876543210fedcba9876543210"});
        fs::write(f.id(), serde_json::to_vec(&current).unwrap()).unwrap();
        fs::write(f.old(), serde_json::to_vec(&legacy).unwrap()).unwrap();
        let before = [fs::read(f.id()).unwrap(), fs::read(f.old()).unwrap()];
        let mut t = f.installation();
        assert!(!t.enabled());
        assert!(t.state().identity_error.unwrap().contains("不一致"));
        assert_eq!(
            [fs::read(f.id()).unwrap(), fs::read(f.old()).unwrap()],
            before
        );
        assert!(t.retry_identity().is_err());
        assert_eq!(
            [fs::read(f.id()).unwrap(), fs::read(f.old()).unwrap()],
            before
        );
        t.reset_install().unwrap();
        assert!(t.enabled());
        assert!(!f.old().exists());
        fs::write(f.id(), serde_json::to_vec(&current).unwrap()).unwrap();
        fs::write(
            f.old(),
            serde_json::to_vec(
                &serde_json::json!({"version":1,"installationId":current["installId"]}),
            )
            .unwrap(),
        )
        .unwrap();
        let merged = f.installation();
        assert!(merged.enabled());
        assert_eq!(merged.install.unwrap().created_at, Some(1793404800));
        assert!(!f.old().exists());
    }

    #[test]
    fn installation_startup_fills_only_allowed_device_fields_and_session_cache_cannot_emit_activity(
    ) {
        let f = Fixture::new();
        let mut t = f.installation();
        t.record("app_started").unwrap();
        let batch = t.take_batch().unwrap();
        let v = payload(&batch);
        let expected = match std::env::consts::OS {
            "macos" => "Mac",
            "windows" => "WindowsPC",
            _ => "LinuxPC",
        };
        assert_eq!(v["device"]["model"], expected);
        assert_eq!(v["device"].as_object().unwrap().len(), 3);
        let mut session = configured(TelemetryMode::Session).with_cache(f.cache());
        let fp = session.identity_fingerprint().unwrap();
        fs::write(f.cache(),serde_json::to_vec(&serde_json::json!({"version":3,"appVersion":env!("CARGO_PKG_VERSION"),"createdAt":now(),"mode":"session","identityFingerprint":fp,"counts":{"app_active":1}})).unwrap()).unwrap();
        session = session.with_cache(f.cache());
        assert!(session.take_batch().is_none());
        assert!(!f.cache().exists());
    }

    #[test]
    fn named_cache_limits_and_failed_deletion_remain_bounded_and_retryable() {
        let f = Fixture::new();
        let t = f.installation();
        let fingerprint = t.identity_fingerprint().unwrap();
        for (version, app_version, created_at) in [
            (3, env!("CARGO_PKG_VERSION"), now() - CACHE_TTL),
            (3, env!("CARGO_PKG_VERSION"), now() + DAY),
            (3, "9.9.9", now()),
            (2, env!("CARGO_PKG_VERSION"), now()),
        ] {
            let saved = serde_json::json!({"version":version,"appVersion":app_version,"createdAt":created_at,"mode":"installation","identityFingerprint":fingerprint,"counts":{"studio_opened":1}});
            fs::write(f.cache(), serde_json::to_vec(&saved).unwrap()).unwrap();
            assert!(f.installation().state().queued_events.is_empty());
            assert!(!f.cache().exists());
        }
        fs::write(f.cache(), vec![b' '; MAX_CACHE_BYTES as usize + 1]).unwrap();
        assert!(f.installation().state().queued_events.is_empty());
        let mut t = f.installation();
        fs::create_dir(f.cache()).unwrap();
        assert!(t.clear_queue().is_err());
        assert!(t.dirty);
        fs::remove_dir(f.cache()).unwrap();
        t.checkpoint();
        assert!(!t.dirty);
        assert!(!f.cache().exists());
        let path = f.0.join("not-created/cache.json");
        let mut t = configured(TelemetryMode::Session).with_cache(path);
        t.record("studio_opened").unwrap();
        t.checkpoint();
        assert!(t.dirty);
        assert!(t.take_batch().is_some());
    }

    #[test]
    fn corrupt_preferences_preserve_all_sidecars_through_the_full_startup_chain() {
        let f = Fixture::new();
        let files = [f.id(), f.old(), f.cache()];
        for path in &files {
            fs::write(path, b"original sidecar fixture").unwrap();
        }
        for configured in [false, true] {
            let t = if configured {
                Telemetry::with_endpoint(
                    Err("fixed read failure".into()),
                    Some("https://stats.example/collect"),
                )
            } else {
                Telemetry::new(Err("fixed read failure".into()))
            };
            let mut t = t.with_install(f.id()).with_cache(f.cache());
            assert!(!t.enabled());
            assert!(t.state().identity_error.is_some());
            t.tick();
            t.checkpoint();
            assert!(t.take_batch().is_none());
            for path in &files {
                assert_eq!(fs::read(path).unwrap(), b"original sidecar fixture");
            }
        }
        let mut explicit = Telemetry::with_endpoint(
            Err("fixed read failure".into()),
            Some("https://stats.example/collect"),
        )
        .with_install(f.id())
        .with_cache(f.cache());
        explicit.set_mode(TelemetryMode::Off);
        for path in &files {
            assert!(!path.exists());
        }
    }

    #[test]
    fn unconfigured_preview_off_and_clear_leave_existing_sidecars_untouched() {
        let f = Fixture::new();
        let files = [f.id(), f.old(), f.cache()];
        for path in &files {
            fs::write(path, b"original preview sidecar").unwrap();
        }
        let mut t = Telemetry::with_endpoint(
            Ok(TelemetryPreferences {
                revision: 4,
                mode: TelemetryMode::Installation,
            }),
            None,
        )
        .with_install(f.id())
        .with_cache(f.cache());
        assert!(t.cache_path.is_none());
        t.set_mode(TelemetryMode::Off);
        t.clear_queue().unwrap();
        t.tick();
        t.checkpoint();
        for path in &files {
            assert_eq!(fs::read(path).unwrap(), b"original preview sidecar");
        }
    }

    #[test]
    fn explicit_off_surfaces_cache_cleanup_failure_and_allows_retry() {
        let f = Fixture::new();
        let mut t = f.installation();
        t.record("studio_opened").unwrap();
        fs::create_dir(f.cache()).unwrap();
        fs::write(f.cache().with_extension("tmp"), b"old temp fixture").unwrap();
        t.set_mode(TelemetryMode::Off);
        assert!(!t.enabled());
        assert!(t.state().cache_error.is_some());
        assert!(t.dirty);
        assert!(
            !f.cache().with_extension("tmp").exists(),
            "all cleanup paths must be attempted"
        );
        let failed_revision = t.state().revision;
        fs::remove_dir(f.cache()).unwrap();
        t.tick();
        assert!(
            t.state().revision > failed_revision,
            "background recovery must become visible to settings"
        );
        t.clear_queue().unwrap();
        assert!(t.state().cache_error.is_none());
        assert!(!t.dirty);
        fs::create_dir(f.cache()).unwrap();
        let stopped = configured(TelemetryMode::Off)
            .with_install(f.id())
            .with_cache(f.cache());
        assert!(!stopped.enabled());
        assert!(stopped.state().cache_error.is_some());
    }

    #[test]
    fn released_boolean_opt_in_and_old_toggle_never_silently_create_installation_tracking() {
        let f = Fixture::new();
        let original = serde_json::json!({"version":1,"installId":"0123456789abcdef0123456789abcdef","createdAt":1793404800_u64});
        fs::write(f.id(), serde_json::to_vec(&original).unwrap()).unwrap();
        let saved: TelemetryPreferences =
            serde_json::from_str(r#"{"revision":4,"enabled":true}"#).unwrap();
        let mut t = Telemetry::with_endpoint(Ok(saved), Some("https://stats.example/collect"))
            .with_install(f.id())
            .with_cache(f.cache());
        assert_eq!(t.mode, TelemetryMode::Session);
        assert!(!f.id().exists());
        t.tick();
        t.record("app_started").unwrap();
        let batch = t.take_batch().unwrap();
        let value = payload(&batch);
        assert_eq!(value["identityMode"], "session");
        assert!(value.get("installId").is_none());
        assert_eq!(batch.envelope.events.len(), 1);
        t.set_enabled(false);
        t.set_enabled(true);
        assert_eq!(t.mode, TelemetryMode::Session);
        assert!(!f.id().exists());
    }

    #[test]
    fn utc_day_formats_calendar_dates() {
        assert_eq!(utc_day(0), "1970-01-01");
        assert_eq!(utc_day(951_782_400), "2000-02-29");
        assert_eq!(utc_day(1793404800), "2026-10-31");
    }

    #[test]
    #[ignore = "Explicit live relay check; requires STARCLIP_TELEMETRY_TEST_ENDPOINT and sends synthetic counts"]
    fn live_relay_accepts_original_fixed_counts() {
        let endpoint = std::env::var("STARCLIP_TELEMETRY_TEST_ENDPOINT")
            .expect("Explicit test relay required");
        for mode in [TelemetryMode::Session, TelemetryMode::Installation] {
            let fixture = Fixture::new();
            if mode == TelemetryMode::Installation {
                // Original, fixed test identity only; never use an existing application directory.
                fs::write(fixture.id(),serde_json::to_vec(&serde_json::json!({"version":1,"installId":"00112233445566778899aabbccddeeff","createdAt":now()})).unwrap()).unwrap();
            }
            let mut t = Telemetry::with_endpoint(
                Ok(TelemetryPreferences { revision: 0, mode }),
                Some(&endpoint),
            )
            .with_install(fixture.id())
            .with_cache(fixture.cache());
            for name in EVENTS.into_iter().filter(|name| *name != "app_active") {
                t.record(name).unwrap();
            }
            t.tick();
            let mut batch = t.take_batch().unwrap();
            batch.envelope.app_version = "0.0.0-telemetry-verification";
            batch.envelope.device = DeviceInfo {
                architecture: "unknown".into(),
                os_major: "unknown".into(),
                model: match std::env::consts::OS {
                    "macos" => "Mac",
                    "windows" => "WindowsPC",
                    _ => "LinuxPC",
                }
                .into(),
            };
            let expected = if mode == TelemetryMode::Installation {
                11
            } else {
                9
            };
            assert_eq!(batch.envelope.events.len(), expected);
            assert!(batch.envelope.events.iter().all(|event| event.count == 1));
            let accepted = send(&batch);
            t.finish(&batch, accepted);
            assert!(
                accepted,
                "synthetic {} sender was not accepted",
                mode.as_str()
            );
            assert!(t.state().queued_events.is_empty());
            println!(
                "Synthetic {} sender accepted: {} predefined event counts; no personal data",
                mode.as_str(),
                expected
            );
        }
    }
}
