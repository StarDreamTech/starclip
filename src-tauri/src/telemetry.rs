//! Optional, bounded feature counters.
//!
//! Two identities are possible and the user picks one explicitly: a random
//! per-run session id, or a random per-installation id stored in a separate
//! file outside the clipboard database. No clipboard data, file paths, window
//! titles or hardware identifiers are ever collected.
use crate::store::Result;
use crate::telemetry_device::DeviceInfo;
use serde::{Deserialize, Deserializer, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use url::Url;

const CACHE_TTL: u64 = 72 * 60 * 60;
const MAX_CACHE_BYTES: u64 = 2048;
const CACHE_VERSION: u8 = 2;
const RETRY_DELAYS: [u64; 4] = [60, 300, 900, 3600];
/// One `app_running` per process start and then once per hour.
const HEARTBEAT_INTERVAL: u64 = 60 * 60;
const IDENTITY_VERSION: u8 = 1;
const MAX_IDENTITY_BYTES: u64 = 512;
/// Lives beside the default data directory, never inside the clipboard library.
pub const IDENTITY_FILE: &str = "usage-identity.json";

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Bounded feature counters. `app_running` is deliberately not part of this
/// list: it is a process heartbeat, never cached and never replayed.
pub const FEATURE_EVENTS: [&str; 9] = [
    "app_started",
    "studio_opened",
    "card_exported",
    "backup_exported",
    "backup_imported",
    "update_check_completed",
    "update_download_completed",
    "update_install_started",
    "font_downloaded",
];
pub const HEARTBEAT_EVENT: &str = "app_running";

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
        // A new installation starts with per-run counting, never with a
        // persistent identifier; `installation` is only ever an explicit choice.
        Self {
            revision: 0,
            mode: TelemetryMode::Session,
        }
    }
}

impl<'de> Deserialize<'de> for TelemetryPreferences {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            revision: u64,
            #[serde(default)]
            mode: Option<String>,
            /// The previous schema stored one boolean switch.
            #[serde(default)]
            enabled: Option<bool>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let mode = match (raw.mode, raw.enabled) {
            (Some(mode), _) => TelemetryMode::parse(&mode)
                .ok_or_else(|| serde::de::Error::custom("统计模式无效"))?,
            (None, Some(false)) => TelemetryMode::Off,
            (None, Some(true)) => TelemetryMode::Session,
            (None, None) => TelemetryMode::Session,
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
    pub mode: &'static str,
    pub enabled: bool,
    pub configured: bool,
    pub endpoint_host: Option<String>,
    pub queued_events: Vec<EventCount>,
    pub last_result: &'static str,
    /// Local identity problem for `installation` mode; never contains the id.
    pub identity_error: Option<&'static str>,
    pub identity_present: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    version: u8,
    /// Present only in `installation` mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    installation_id: Option<String>,
    session_id: String,
    app_version: &'static str,
    platform: &'static str,
    device: DeviceInfo,
    events: Vec<EventCount>,
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
    /// Feature counts only; a heartbeat is never written to disk.
    counts: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityFile {
    version: u8,
    installation_id: String,
}

struct InFlight {
    session_id: String,
    generation: u64,
    created_at: u64,
    counts: [u8; FEATURE_EVENTS.len()],
}

pub struct Telemetry {
    endpoint: Option<Url>,
    mode: TelemetryMode,
    session_id: Option<String>,
    installation_id: Option<String>,
    identity_path: Option<PathBuf>,
    identity_error: Option<&'static str>,
    revision: u64,
    counts: [u8; FEATURE_EVENTS.len()],
    heartbeat: bool,
    next_heartbeat_at: u64,
    in_flight: Option<InFlight>,
    last_result: &'static str,
    device: DeviceInfo,
    cache_path: Option<PathBuf>,
    created_at: Option<u64>,
    generation: u64,
    failures: usize,
    next_attempt_at: u64,
    dirty: bool,
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

fn random_hex_16() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成临时统计会话，请重试。".to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn valid_identity(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Reads the installation identity without following symlinks or trusting size.
fn read_identity(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_IDENTITY_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_IDENTITY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_IDENTITY_BYTES {
        return None;
    }
    let saved: IdentityFile = serde_json::from_slice(&bytes).ok()?;
    (saved.version == IDENTITY_VERSION && valid_identity(&saved.installation_id))
        .then_some(saved.installation_id)
}

/// Atomic replacement, owner-only permissions, and never through a symlink.
fn write_identity(path: &Path, value: &str) -> Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err("统计标识文件不能是符号链接。".into());
    }
    let temporary = path.with_extension("tmp");
    let _ = fs::remove_file(&temporary);
    let bytes = serde_json::to_vec(&IdentityFile {
        version: IDENTITY_VERSION,
        installation_id: value.to_owned(),
    })
    .map_err(|_| "统计标识未能写入。".to_string())?;
    let written = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    let _ = fs::remove_file(&temporary);
    written.map_err(|_| "统计标识未能保存，已停止按安装统计。".to_string())
}

fn remove_identity(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("统计标识未能删除；请检查目录写入权限后重试。".into()),
    }
}

impl Telemetry {
    pub fn new(preferences: Result<TelemetryPreferences>) -> Self {
        let mut telemetry =
            Self::with_endpoint(preferences, option_env!("STARCLIP_TELEMETRY_ENDPOINT"));
        if telemetry.enabled() {
            telemetry.device = DeviceInfo::detect();
        }
        telemetry
    }

    fn with_endpoint(preferences: Result<TelemetryPreferences>, value: Option<&str>) -> Self {
        // Only the missing-key path may choose the product default. Corrupt or
        // unreadable preferences must never accidentally turn collection on.
        let preferences = preferences.unwrap_or(TelemetryPreferences {
            revision: 0,
            mode: TelemetryMode::Off,
        });
        let endpoint = endpoint(value);
        let session_id = random_hex_16().ok();
        let mode = preferences.mode;
        let mut telemetry = Self {
            last_result: if endpoint.is_none() {
                "unconfigured"
            } else if mode == TelemetryMode::Off {
                "disabled"
            } else {
                "never"
            },
            endpoint,
            mode,
            session_id,
            installation_id: None,
            identity_path: None,
            identity_error: None,
            revision: preferences.revision,
            counts: [0; FEATURE_EVENTS.len()],
            heartbeat: false,
            next_heartbeat_at: 0,
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
        };
        if telemetry.enabled() {
            telemetry.arm_heartbeat();
        }
        telemetry
    }

    /// The identity file lives beside the default data directory, so moving the
    /// clipboard library neither carries it along nor deletes it.
    pub fn with_identity(mut self, path: PathBuf) -> Self {
        self.identity_path = Some(path);
        if self.mode == TelemetryMode::Installation {
            if self.ensure_identity().is_ok() && self.next_heartbeat_at == 0 {
                self.arm_heartbeat();
            }
        }
        self
    }

    /// A tiny independent cache outside clipboard backups. No session/device IDs on disk.
    pub fn with_cache(mut self, path: PathBuf) -> Self {
        self.cache_path = Some(path.clone());
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
            (saved.version == CACHE_VERSION
                && saved.counts.len() == FEATURE_EVENTS.len()
                && saved.app_version == env!("CARGO_PKG_VERSION")
                && age < CACHE_TTL
                && saved.counts.iter().all(|&n| n <= 100))
            .then_some(saved)
        })();
        if let Some(saved) = restored {
            self.counts.copy_from_slice(&saved.counts);
            self.created_at = Some(saved.created_at);
        } else {
            let _ = fs::remove_file(&path);
        }
        let _ = fs::remove_file(path.with_extension("tmp"));
        self
    }

    fn enabled(&self) -> bool {
        self.mode != TelemetryMode::Off
            && self.endpoint.is_some()
            && self.session_id.is_some()
            && (self.mode != TelemetryMode::Installation || self.installation_id.is_some())
    }

    fn arm_heartbeat(&mut self) {
        self.heartbeat = true;
        self.next_heartbeat_at = now().saturating_add(HEARTBEAT_INTERVAL);
    }

    /// Creates or reloads the local installation identity. On failure the mode
    /// stops sending and the reason is surfaced in settings; the id never leaves
    /// this process except inside the analytics payload.
    fn ensure_identity(&mut self) -> Result<()> {
        if self.mode != TelemetryMode::Installation {
            self.installation_id = None;
            self.identity_error = None;
            return Ok(());
        }
        let Some(path) = self.identity_path.clone() else {
            self.installation_id = None;
            self.identity_error = Some("未配置统计标识位置，已停止按安装统计。");
            return Err(self.identity_error.unwrap().into());
        };
        if let Some(current) = &self.installation_id {
            if read_identity(&path).as_deref() == Some(current.as_str()) {
                self.identity_error = None;
                return Ok(());
            }
        }
        let value = match read_identity(&path) {
            Some(saved) => saved,
            None => {
                let created = random_hex_16()?;
                if let Err(error) = write_identity(&path, &created) {
                    self.installation_id = None;
                    self.identity_error = Some("统计标识未能保存，已停止按安装统计。");
                    return Err(error);
                }
                created
            }
        };
        self.installation_id = Some(value);
        self.identity_error = None;
        Ok(())
    }

    /// Switching mode always drops pending counts and the heartbeat, so nothing
    /// measured under the previous identity can be sent under the next one.
    pub fn set_mode(&mut self, mode: TelemetryMode) {
        let was_installation = self.mode == TelemetryMode::Installation;
        self.clear();
        self.mode = mode;
        // A new measurement session: nothing from the previous mode can match
        // the new session id, even if a stale callback returns late.
        self.session_id = random_hex_16().ok();
        if mode == TelemetryMode::Installation {
            // A local failure keeps the choice but stops sending until it is fixed.
            let _ = self.ensure_identity();
        } else {
            self.installation_id = None;
            self.identity_error = None;
            if was_installation {
                if let Some(path) = self.identity_path.clone() {
                    if remove_identity(&path).is_err() {
                        self.identity_error = Some("统计标识未能删除；请检查目录写入权限后重试。");
                    }
                }
            }
        }
        self.last_result = if self.endpoint.is_none() {
            "unconfigured"
        } else if self.enabled() {
            "never"
        } else {
            "disabled"
        };
        if self.enabled() {
            self.arm_heartbeat();
        }
        self.bump();
    }

    /// Replaces the local installation identity and drops everything pending.
    pub fn reset_identity(&mut self) -> Result<()> {
        if self.mode != TelemetryMode::Installation {
            return Err("只有按安装统计时才需要重置标识。".into());
        }
        let Some(path) = self.identity_path.clone() else {
            return Err("未配置统计标识位置。".into());
        };
        remove_identity(&path)?;
        self.installation_id = None;
        self.identity_error = None;
        self.clear();
        self.ensure_identity()?;
        self.arm_heartbeat();
        self.bump();
        Ok(())
    }

    fn expire(&mut self, at: u64) {
        if self
            .created_at
            .is_some_and(|start| at < start || at - start >= CACHE_TTL)
        {
            self.counts = [0; FEATURE_EVENTS.len()];
            self.created_at = None;
            self.dirty = true;
            self.bump();
        }
    }

    /// Called by the existing minute timer and at normal exit, never by a UI
    /// network wait. It also arms the hourly heartbeat, including at most one
    /// catch-up after the machine wakes from sleep.
    pub fn tick(&mut self) {
        let at = now();
        self.expire(at);
        if !self.enabled() {
            self.heartbeat = false;
            self.next_heartbeat_at = 0;
            return;
        }
        if self.next_heartbeat_at == 0 {
            self.arm_heartbeat();
        } else if at >= self.next_heartbeat_at {
            self.heartbeat = true;
            self.next_heartbeat_at = at.saturating_add(HEARTBEAT_INTERVAL);
        }
        self.checkpoint();
    }

    pub fn checkpoint(&mut self) {
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
            let removed = [path.clone(), path.with_extension("tmp")]
                .iter()
                .all(|file| match fs::remove_file(file) {
                    Ok(()) => true,
                    Err(error) => error.kind() == std::io::ErrorKind::NotFound,
                });
            self.dirty = !removed;
            return;
        }
        let saved = CachedCounts {
            version: CACHE_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            created_at: created_at.unwrap_or_else(now),
            counts: counts.to_vec(),
        };
        let temporary = path.with_extension("tmp");
        let written = (|| -> std::io::Result<()> {
            let bytes = serde_json::to_vec(&saved)?;
            let _ = fs::remove_file(&temporary);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            file.write_all(&bytes)?;
            drop(file);
            fs::rename(&temporary, path)
        })();
        if written.is_ok() {
            self.dirty = false;
        }
        let _ = fs::remove_file(temporary);
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn state(&self) -> TelemetryState {
        TelemetryState {
            revision: self.revision,
            mode: self.mode.as_str(),
            enabled: self.enabled(),
            configured: self.endpoint.is_some(),
            endpoint_host: self
                .endpoint
                .as_ref()
                .and_then(|url| url.host_str().map(str::to_owned)),
            queued_events: FEATURE_EVENTS
                .iter()
                .zip(self.counts)
                .filter_map(|(&name, count)| (count > 0).then_some(EventCount { name, count }))
                .collect(),
            last_result: self.last_result,
            identity_error: self.identity_error,
            identity_present: self.installation_id.is_some(),
        }
    }

    pub fn validate_configured(&mut self) -> Result<()> {
        if self.endpoint.is_none() {
            return Err("此版本尚未配置统计接收服务，无法开启上报。".into());
        }
        if self.session_id.is_none() {
            self.session_id = Some(random_hex_16()?);
        }
        #[cfg(not(test))]
        if self.device.architecture == "unknown" {
            self.device = DeviceInfo::detect();
        }
        Ok(())
    }

    pub fn record(&mut self, name: &str) -> Result<()> {
        let index = FEATURE_EVENTS
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
        self.counts = [0; FEATURE_EVENTS.len()];
        self.created_at = None;
        self.heartbeat = false;
        self.next_heartbeat_at = 0;
        self.generation = self.generation.wrapping_add(1);
        self.failures = 0;
        self.next_attempt_at = 0;
        self.dirty = true;
        self.checkpoint();
        self.bump();
    }

    pub fn clear_queue(&mut self) -> Result<()> {
        self.clear();
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
        let mut events = self.state().queued_events;
        if self.heartbeat {
            events.push(EventCount {
                name: HEARTBEAT_EVENT,
                count: 1,
            });
        }
        let batch = Batch {
            endpoint: self.endpoint.clone()?,
            envelope: Envelope {
                version: 3,
                installation_id: (self.mode == TelemetryMode::Installation)
                    .then(|| self.installation_id.clone())
                    .flatten(),
                session_id: session_id.clone(),
                app_version: env!("CARGO_PKG_VERSION"),
                platform: std::env::consts::OS,
                device: self.device.clone(),
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
        self.counts = [0; FEATURE_EVENTS.len()];
        self.created_at = None;
        // A delivered or failed heartbeat is never replayed.
        self.heartbeat = false;
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

    fn configured() -> Telemetry {
        Telemetry::with_endpoint(
            Ok(TelemetryPreferences::default()),
            Some("https://stats.example/collect"),
        )
    }

    fn installation() -> Telemetry {
        Telemetry::with_endpoint(
            Ok(TelemetryPreferences {
                revision: 1,
                mode: TelemetryMode::Installation,
            }),
            Some("https://stats.example/collect"),
        )
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("starclip-usage-{}", random_hex_16().unwrap()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn cache(&self) -> PathBuf {
            self.0.join("usage-counts.json")
        }
        fn identity(&self) -> PathBuf {
            self.0.join(IDENTITY_FILE)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_and_endpoint_validation_prevent_unconfigured_collection() {
        for value in [
            None,
            Some("http://stats.example/collect"),
            Some("https://user:secret@stats.example/collect"),
            Some("https://stats.example/collect?secret=x"),
            Some("https://stats.example/collect#x"),
            Some("https://stats.example/other"),
        ] {
            let mut telemetry = Telemetry::with_endpoint(
                Ok(TelemetryPreferences {
                    revision: 1,
                    mode: TelemetryMode::Session,
                }),
                value,
            );
            assert!(!telemetry.state().configured);
            assert!(!telemetry.state().enabled);
            assert!(telemetry.validate_configured().is_err());
            telemetry.record("app_started").unwrap();
            assert!(telemetry.take_batch().is_none());
            assert!(telemetry.state().queued_events.is_empty());
        }
    }

    #[test]
    fn configured_default_is_enabled_but_saved_opt_out_and_read_errors_stay_disabled() {
        let mut telemetry = configured();
        assert!(telemetry.state().enabled);
        assert_eq!(telemetry.state().mode, "session");
        telemetry.record("app_started").unwrap();
        assert_eq!(
            telemetry.state().queued_events,
            vec![EventCount {
                name: "app_started",
                count: 1
            }]
        );

        for preferences in [
            Ok(TelemetryPreferences {
                revision: 4,
                mode: TelemetryMode::Off,
            }),
            Err("fixed preference read error".into()),
        ] {
            let mut telemetry =
                Telemetry::with_endpoint(preferences, Some("https://stats.example/collect"));
            assert!(telemetry.state().configured);
            assert!(!telemetry.state().enabled);
            telemetry.record("update_download_completed").unwrap();
            telemetry.record("update_install_started").unwrap();
            assert!(telemetry.state().queued_events.is_empty());
            assert!(telemetry.take_batch().is_none());
        }
    }

    #[test]
    fn legacy_boolean_preference_maps_off_and_session_without_inventing_installation() {
        let parse = |json: &str| serde_json::from_str::<TelemetryPreferences>(json);
        assert_eq!(
            parse(r#"{"revision":3,"enabled":false}"#).unwrap().mode,
            TelemetryMode::Off
        );
        assert_eq!(
            parse(r#"{"revision":3,"enabled":true}"#).unwrap().mode,
            TelemetryMode::Session
        );
        assert_eq!(
            parse(r#"{"revision":5,"mode":"installation"}"#)
                .unwrap()
                .mode,
            TelemetryMode::Installation
        );
        assert_eq!(
            parse(r#"{"revision":5,"mode":"session"}"#).unwrap().mode,
            TelemetryMode::Session
        );
        assert!(parse(r#"{"revision":5,"mode":"forever"}"#).is_err());
        assert!(parse(r#"{"revision":5,"mode":"off","extra":1}"#).is_err());
        assert!(
            parse(r#"{"mode":"off"}"#).is_err(),
            "revision stays mandatory"
        );
        // Writing back always uses the explicit mode and never the old switch.
        let json = serde_json::to_value(TelemetryPreferences {
            revision: 7,
            mode: TelemetryMode::Installation,
        })
        .unwrap();
        assert_eq!(json.as_object().unwrap().len(), 2);
        assert_eq!(json["mode"], "installation");
        assert!(json.get("enabled").is_none());
        assert_eq!(TelemetryPreferences::default().mode, TelemetryMode::Session);
    }

    #[test]
    fn only_bounded_allowlisted_counts_enter_the_payload_and_retries_back_off() {
        let mut telemetry = configured();
        telemetry.validate_configured().unwrap();
        assert!(telemetry.record_frontend("card_exported").is_err());
        assert!(telemetry
            .record_frontend("update_download_completed")
            .is_err());
        assert!(telemetry.record_frontend("update_install_started").is_err());
        assert!(telemetry.record("clipboard_secret").is_err());
        // The heartbeat is not a feature event and cannot be incremented by hand.
        assert!(telemetry.record(HEARTBEAT_EVENT).is_err());
        for _ in 0..110 {
            telemetry.record_frontend("studio_opened").unwrap();
        }
        for event in FEATURE_EVENTS {
            telemetry.record(event).unwrap();
        }
        let batch = telemetry.take_batch().unwrap();
        let payload = serde_json::to_value(&batch.envelope).unwrap();
        assert_eq!(payload.as_object().unwrap().len(), 6);
        assert!(payload.get("installationId").is_none());
        assert_eq!(payload["version"], 3);
        assert_eq!(payload["device"].as_object().unwrap().len(), 3);
        assert_eq!(batch.envelope.events.len(), FEATURE_EVENTS.len() + 1);
        assert_eq!(batch.envelope.events[1].count, 100);
        assert_eq!(batch.envelope.events.last().unwrap().name, HEARTBEAT_EVENT);
        assert_eq!(batch.envelope.session_id.len(), 32);
        assert!(batch
            .envelope
            .session_id
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
        assert!(telemetry.take_batch().is_none());
        telemetry.finish(&batch, false);
        assert_eq!(telemetry.state().last_result, "failed");
        assert_eq!(
            telemetry.state().queued_events.len(),
            FEATURE_EVENTS.len(),
            "a failed heartbeat is never replayed"
        );
        assert!(telemetry.take_batch().is_none());
        for delay in [300, 900, 3600, 3600] {
            telemetry.next_attempt_at = 0;
            let retry = telemetry.take_batch().unwrap();
            telemetry.record("studio_opened").unwrap();
            telemetry.finish(&retry, false);
            assert!((now() + delay - 1..=now() + delay).contains(&telemetry.next_attempt_at));
            assert_eq!(telemetry.counts[1], 100);
        }
        telemetry.next_attempt_at = 0;
        let retry = telemetry.take_batch().unwrap();
        telemetry.finish(&retry, true);
        assert!(telemetry.state().queued_events.is_empty());
        assert_eq!(telemetry.failures, 0);
    }

    #[test]
    fn switching_mode_clears_pending_rotates_identity_and_ignores_stale_completion() {
        let mut telemetry = configured();
        telemetry.record("app_started").unwrap();
        let batch = telemetry.take_batch().unwrap();
        telemetry.record("card_exported").unwrap();
        telemetry.set_mode(TelemetryMode::Off);
        assert_ne!(
            telemetry.session_id.as_ref(),
            Some(&batch.envelope.session_id)
        );
        assert!(telemetry.state().queued_events.is_empty());
        assert!(!telemetry.ready());
        telemetry.finish(&batch, true);
        assert!(!telemetry.state().enabled);
        assert_eq!(telemetry.state().mode, "off");
        assert_eq!(telemetry.state().last_result, "disabled");
        telemetry.set_mode(TelemetryMode::Session);
        assert_eq!(telemetry.state().last_result, "never");
        assert!(telemetry.state().enabled);
        telemetry.record("studio_opened").unwrap();
        telemetry.clear();
        assert!(telemetry.take_batch().is_none());
    }

    #[test]
    fn heartbeat_starts_once_per_hour_and_catches_up_at_most_once() {
        let fixture = Fixture::new();
        let mut telemetry = configured().with_cache(fixture.cache());
        assert!(telemetry.heartbeat, "a start arms one heartbeat");
        telemetry.tick();
        assert!(telemetry.heartbeat);
        // Not due yet: a tick changes nothing.
        telemetry.next_heartbeat_at = now() + 600;
        telemetry.tick();
        assert!(telemetry.heartbeat);
        // After a long sleep only one catch-up is recorded.
        telemetry.heartbeat = false;
        telemetry.next_heartbeat_at = now() - 5 * HEARTBEAT_INTERVAL;
        telemetry.tick();
        assert!(telemetry.heartbeat);
        assert!(
            (now() + HEARTBEAT_INTERVAL - 2..=now() + HEARTBEAT_INTERVAL)
                .contains(&telemetry.next_heartbeat_at)
        );
        let batch = telemetry.take_batch().unwrap();
        assert_eq!(batch.envelope.events.len(), 1);
        assert_eq!(batch.envelope.events[0].name, HEARTBEAT_EVENT);
        assert!(!telemetry.heartbeat, "taking the batch consumes it");
        // No feature counts and no pending heartbeat means no request at all.
        assert!(!telemetry.ready());
        assert!(telemetry.take_batch().is_none());
        // A heartbeat is never written to the offline cache.
        assert!(!fixture.cache().exists());
        // Resetting clears the heartbeat and the schedule.
        telemetry.heartbeat = true;
        telemetry.clear();
        assert!(!telemetry.heartbeat);
        assert_eq!(telemetry.next_heartbeat_at, 0);
    }

    #[test]
    fn heartbeat_is_dropped_when_collection_stops() {
        let mut telemetry = configured();
        assert!(telemetry.heartbeat);
        telemetry.set_mode(TelemetryMode::Off);
        assert!(!telemetry.heartbeat);
        assert_eq!(telemetry.next_heartbeat_at, 0);
        telemetry.tick();
        assert!(!telemetry.heartbeat);
        assert!(telemetry.take_batch().is_none());
    }

    #[test]
    fn installation_identity_persists_across_restarts_and_never_leaks_into_state() {
        let fixture = Fixture::new();
        let first = installation().with_identity(fixture.identity());
        assert!(first.state().enabled);
        assert!(first.state().identity_present);
        assert_eq!(first.state().identity_error, None);
        let id = first.installation_id.clone().unwrap();
        assert!(valid_identity(&id));
        assert_eq!(
            read_identity(&fixture.identity()).as_deref(),
            Some(id.as_str())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(fixture.identity())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let bytes = fs::read(fixture.identity()).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved.as_object().unwrap().len(), 2);
        assert_eq!(saved["version"], 1);
        // The full id is never handed to the UI or written into logs.
        assert!(!serde_json::to_string(&first.state()).unwrap().contains(&id));

        // A restart reuses the stored identity instead of rotating it.
        let second = installation().with_identity(fixture.identity());
        assert_eq!(second.installation_id.as_deref(), Some(id.as_str()));
        // Upgrading the app version does not change the identity.
        assert_eq!(
            read_identity(&fixture.identity()).as_deref(),
            Some(id.as_str())
        );
    }

    #[test]
    fn installation_payload_carries_the_identity_and_a_third_version() {
        let fixture = Fixture::new();
        let mut telemetry = installation().with_identity(fixture.identity());
        let id = telemetry.installation_id.clone().unwrap();
        telemetry.record("app_started").unwrap();
        let batch = telemetry.take_batch().unwrap();
        let payload = serde_json::to_value(&batch.envelope).unwrap();
        assert_eq!(payload.as_object().unwrap().len(), 7);
        assert_eq!(payload["version"], 3);
        assert_eq!(payload["installationId"], id);
        assert!(payload.get("userId").is_none());
        let text = serde_json::to_string(&payload).unwrap();
        for forbidden in [
            "/Users/",
            "clipboard",
            "content",
            "title",
            "hostname",
            "serial",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} must never be sent");
        }
    }

    #[test]
    fn identity_failures_stop_installation_sending_and_surface_a_local_error() {
        let fixture = Fixture::new();
        // A directory in place of the file cannot be read or replaced.
        fs::create_dir(fixture.identity()).unwrap();
        let mut telemetry = installation().with_identity(fixture.identity());
        assert!(telemetry.state().identity_error.is_some());
        assert!(!telemetry.state().identity_present);
        assert!(!telemetry.state().enabled);
        telemetry.record("app_started").unwrap();
        assert!(telemetry.take_batch().is_none());
        assert!(telemetry.state().queued_events.is_empty());
        // The session mode still sends even though the leftover path is unusable.
        telemetry.set_mode(TelemetryMode::Session);
        assert!(telemetry.state().enabled);
        assert!(
            telemetry.state().identity_error.is_some(),
            "a path that cannot be removed is reported instead of hidden"
        );
        assert!(!telemetry.state().identity_present);
    }

    #[cfg(unix)]
    #[test]
    fn identity_symlinks_are_refused_and_never_followed() {
        let fixture = Fixture::new();
        let target = fixture.0.join("original-fixture.txt");
        fs::write(&target, b"original fixed fixture").unwrap();
        std::os::unix::fs::symlink(&target, fixture.identity()).unwrap();
        let mut telemetry = installation().with_identity(fixture.identity());
        assert!(telemetry.state().identity_error.is_some());
        assert!(!telemetry.state().identity_present);
        assert_eq!(fs::read(&target).unwrap(), b"original fixed fixture");
        assert!(fs::symlink_metadata(fixture.identity())
            .unwrap()
            .file_type()
            .is_symlink());
        // Leaving the mode removes the link, never the target it points at.
        telemetry.set_mode(TelemetryMode::Session);
        assert!(!fs::symlink_metadata(fixture.identity()).is_ok());
        assert_eq!(fs::read(&target).unwrap(), b"original fixed fixture");
    }

    #[test]
    fn leaving_installation_deletes_the_identity_and_clears_pending() {
        let fixture = Fixture::new();
        let mut telemetry = installation().with_identity(fixture.identity());
        assert!(fixture.identity().exists());
        telemetry.record("app_started").unwrap();
        telemetry.set_mode(TelemetryMode::Session);
        assert!(!fixture.identity().exists());
        assert!(telemetry.installation_id.is_none());
        assert!(!telemetry.state().identity_present);
        assert!(telemetry.state().queued_events.is_empty());
        assert_eq!(telemetry.state().mode, "session");
        // Nothing installation-specific survives in the session payload.
        telemetry.record("app_started").unwrap();
        let batch = telemetry.take_batch().unwrap();
        assert!(batch.envelope.installation_id.is_none());
    }

    #[test]
    fn resetting_the_identity_rotates_it_and_drops_pending_counts() {
        let fixture = Fixture::new();
        let mut telemetry = installation().with_identity(fixture.identity());
        let first = telemetry.installation_id.clone().unwrap();
        telemetry.record("app_started").unwrap();
        telemetry.reset_identity().unwrap();
        let second = telemetry.installation_id.clone().unwrap();
        assert_ne!(first, second);
        assert!(valid_identity(&second));
        assert_eq!(
            read_identity(&fixture.identity()).as_deref(),
            Some(second.as_str())
        );
        assert!(telemetry.state().queued_events.is_empty());
        // Session mode has no persistent identity to reset.
        telemetry.set_mode(TelemetryMode::Session);
        assert!(telemetry.reset_identity().is_err());
    }

    #[test]
    fn offline_cache_restores_counts_without_identity_and_success_removes_it() {
        let fixture = Fixture::new();
        let mut telemetry = configured().with_cache(fixture.cache());
        telemetry.record("studio_opened").unwrap();
        let batch = telemetry.take_batch().unwrap();
        telemetry.record("card_exported").unwrap();
        telemetry.finish(&batch, false);
        let bytes = fs::read(fixture.cache()).unwrap();
        assert!(bytes.len() < MAX_CACHE_BYTES as usize);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 4);
        assert_eq!(value["version"], CACHE_VERSION);
        assert_eq!(
            value["counts"].as_array().unwrap().len(),
            FEATURE_EVENTS.len()
        );
        assert!(value.get("sessionId").is_none());
        assert!(value.get("device").is_none());
        assert!(value.get("appRunning").is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(fixture.cache()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let mut restored = configured().with_cache(fixture.cache());
        assert_ne!(restored.session_id, telemetry.session_id);
        assert_eq!(
            restored.state().queued_events,
            telemetry.state().queued_events
        );
        let batch = restored.take_batch().unwrap();
        restored.finish(&batch, true);
        assert!(!fixture.cache().exists());
        assert!(restored.state().queued_events.is_empty());
    }

    #[test]
    fn cache_rejects_corruption_expiry_foreign_version_and_unexpected_fields() {
        let fixture = Fixture::new();
        let counts: Vec<u8> = std::iter::once(1)
            .chain(std::iter::repeat(0))
            .take(FEATURE_EVENTS.len())
            .collect();
        let valid = serde_json::json!({
            "version": CACHE_VERSION, "appVersion": env!("CARGO_PKG_VERSION"),
            "createdAt": now(), "counts": counts
        });
        for (key, value) in [
            ("createdAt", serde_json::json!(now() - CACHE_TTL)),
            ("createdAt", serde_json::json!(now() + 100)),
            ("appVersion", serde_json::json!("0.0.0-fixture")),
            ("version", serde_json::json!(1)),
            ("counts", serde_json::json!([101, 0, 0, 0, 0, 0, 0, 0, 0])),
            // A shorter array from an older build must not shift positions.
            ("counts", serde_json::json!([1, 0, 0])),
            ("sessionId", serde_json::json!("fixture-not-allowed")),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            fs::write(fixture.cache(), serde_json::to_vec(&invalid).unwrap()).unwrap();
            let restored = configured().with_cache(fixture.cache());
            assert!(restored.state().queued_events.is_empty(), "{key}");
            assert!(!fixture.cache().exists());
        }
        for bytes in [
            b"not-json".to_vec(),
            vec![b' '; MAX_CACHE_BYTES as usize + 1],
        ] {
            fs::write(fixture.cache(), bytes).unwrap();
            assert!(configured()
                .with_cache(fixture.cache())
                .state()
                .queued_events
                .is_empty());
            assert!(!fixture.cache().exists());
        }
    }

    #[test]
    fn clearing_and_disabling_forget_cached_and_in_flight_counts() {
        let fixture = Fixture::new();
        for disable in [false, true] {
            let mut telemetry = configured().with_cache(fixture.cache());
            telemetry.record("app_started").unwrap();
            let batch = telemetry.take_batch().unwrap();
            assert!(fixture.cache().exists());
            if disable {
                telemetry.set_mode(TelemetryMode::Off);
            } else {
                telemetry.clear_queue().unwrap();
            }
            assert!(!fixture.cache().exists());
            telemetry.finish(&batch, false);
            assert!(!fixture.cache().exists());
            assert!(telemetry.state().queued_events.is_empty());
        }
        let mut telemetry = configured().with_cache(fixture.cache());
        telemetry.record("studio_opened").unwrap();
        telemetry.checkpoint();
        let restored = Telemetry::with_endpoint(
            Ok(TelemetryPreferences {
                revision: 1,
                mode: TelemetryMode::Off,
            }),
            Some("https://stats.example/collect"),
        )
        .with_cache(fixture.cache());
        assert!(restored.state().queued_events.is_empty());
        assert!(!fixture.cache().exists());
    }

    #[test]
    fn cache_write_failures_stay_bounded_and_failed_deletions_remain_retryable() {
        let fixture = Fixture::new();
        let path = fixture.0.join("missing").join("usage-counts.json");
        let mut telemetry = configured().with_cache(path);
        for _ in 0..200 {
            telemetry.record("app_started").unwrap();
        }
        telemetry.checkpoint();
        assert!(telemetry.dirty);
        assert_eq!(telemetry.counts[0], 100);
        assert!(
            telemetry.take_batch().is_some(),
            "cache failure cannot block use or network delivery"
        );

        let mut telemetry = configured().with_cache(fixture.cache());
        fs::create_dir(fixture.cache()).unwrap();
        assert!(telemetry.clear_queue().is_err());
        assert!(telemetry.dirty);
        fs::remove_dir(fixture.cache()).unwrap();
        telemetry.checkpoint();
        assert!(!telemetry.dirty);
        telemetry.record("app_started").unwrap();
        telemetry.created_at = Some(now() - CACHE_TTL);
        telemetry.checkpoint();
        assert!(!fixture.cache().exists());
        assert!(telemetry.take_batch().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cache_symlinks_are_not_read_or_followed() {
        let fixture = Fixture::new();
        let target = fixture.0.join("original-fixture.txt");
        fs::write(&target, b"original fixed fixture").unwrap();
        std::os::unix::fs::symlink(&target, fixture.cache()).unwrap();
        let mut telemetry = configured().with_cache(fixture.cache());
        assert!(telemetry.state().queued_events.is_empty());
        telemetry.record("app_started").unwrap();
        telemetry.checkpoint();
        assert_eq!(fs::read(target).unwrap(), b"original fixed fixture");
        assert!(!fs::symlink_metadata(fixture.cache())
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    #[ignore = "Explicit live relay check; requires STARCLIP_TELEMETRY_TEST_ENDPOINT and sends synthetic counts"]
    fn live_relay_accepts_original_fixed_counts() {
        let endpoint = std::env::var("STARCLIP_TELEMETRY_TEST_ENDPOINT")
            .expect("Set STARCLIP_TELEMETRY_TEST_ENDPOINT to the test relay HTTPS /collect URL");
        let mut telemetry =
            Telemetry::with_endpoint(Ok(TelemetryPreferences::default()), Some(&endpoint));
        telemetry.validate_configured().unwrap();
        assert!(telemetry.state().enabled, "configured default must be on");
        for event in FEATURE_EVENTS {
            telemetry.record(event).unwrap();
        }
        let mut batch = telemetry.take_batch().unwrap();
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
        // Only this test substitutes a clearly synthetic version. Production
        // batching and the real HTTPS sender remain unchanged; no store is opened.
        batch.envelope.app_version = "0.0.0-telemetry-verification";
        assert_eq!(batch.envelope.events.len(), FEATURE_EVENTS.len() + 1);
        assert!(batch.envelope.events.iter().all(|event| event.count == 1));
        let accepted = send(&batch);
        telemetry.finish(&batch, accepted);
        assert!(accepted, "relay did not return HTTP 204; no retry is made");
        assert_eq!(telemetry.state().last_result, "sent");
        assert!(telemetry.take_batch().is_none());

        telemetry.record("studio_opened").unwrap();
        telemetry.set_mode(TelemetryMode::Off);
        for event in FEATURE_EVENTS {
            telemetry.record(event).unwrap();
        }
        assert!(!telemetry.state().enabled);
        assert!(telemetry.state().queued_events.is_empty());
        assert!(telemetry.take_batch().is_none());
        // HTTP acceptance does not prove ingestion into either provider's reports.
    }
}
