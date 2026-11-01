use crate::appearance::{self, Appearance};
use crate::images::{self, ImageMetadata, PreparedImage};
use crate::privacy::{self, Metadata, PrivacyRescan, PrivacyRules};
use crate::starpet_growth::{GrowthChange, GrowthState, MAX_REVISION};
use crate::telemetry::{TelemetryMode, TelemetryPreferences};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub type Result<T> = std::result::Result<T, String>;
pub const SCHEMA_VERSION: u32 = 3;
pub const MAX_CONTENT_BYTES: usize = 256 * 1024;
const MAX_BACKUP_BYTES: usize = 20 * 1024 * 1024;
const MAX_BACKUP_CLIPS: usize = 10_000;
const MAX_BACKUP_GROUPS: usize = 1000;
const MAX_BACKUP_IMAGE_PIXELS: u64 = 256_000_000;

fn check_backup_limits(bytes: usize, clips: usize, groups: usize) -> Result<()> {
    if bytes > MAX_BACKUP_BYTES || clips > MAX_BACKUP_CLIPS || groups > MAX_BACKUP_GROUPS {
        return Err("JSON 备份上限为 20 MiB、10000 条星笺和 1000 个分组；超限时可在数据设置中迁移完整数据目录，无需删除记录。".into());
    }
    Ok(())
}

fn check_backup_image_limits(clips: &[Clip]) -> Result<()> {
    let pixels = clips
        .iter()
        .filter_map(|clip| clip.image.as_ref())
        .fold(0u64, |sum, image| {
            sum.saturating_add(u64::from(image.width) * u64::from(image.height))
        });
    if pixels > MAX_BACKUP_IMAGE_PIXELS {
        return Err("JSON 备份中的图片累计不能超过 2.56 亿像素；请使用数据设置中的完整数据目录迁移，无需删除记录。".into());
    }
    Ok(())
}

pub const HISTORY_PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HistoryQuery {
    pub page: u32,
    pub filter: String,
    pub query: String,
    pub sources: Vec<String>,
    pub tag: String,
    pub privacy: String,
    pub ai_eligible: bool,
}
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            page: 1,
            filter: "all".into(),
            query: String::new(),
            sources: vec![],
            tag: String::new(),
            privacy: "all".into(),
            ai_eligible: false,
        }
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySummary {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub counts: BTreeMap<String, u64>,
    pub group_counts: BTreeMap<i64, u64>,
    pub sources: Vec<String>,
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
}
pub struct HistoryPage {
    pub clips: Vec<Clip>,
    pub history: HistorySummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEntryKind {
    File,
    Directory,
    Unknown,
}

impl FileEntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Unknown => "unknown",
        }
    }
}

/// One referenced path. The record stores references only; file contents are
/// never read and folders are never scanned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub entry_kind: FileEntryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    #[serde(skip)]
    pub record_identity: String,
    pub id: i64,
    pub content: String,
    pub title: String,
    pub kind: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<FileEntry>>,
    pub pinned: bool,
    #[serde(default)]
    pub keep_forever: bool,
    pub group_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub copy_count: i64,
    #[serde(default)]
    pub privacy_manual: bool,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub privacy_reasons: Vec<String>,
    #[serde(default)]
    pub scan_version: u32,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub manual_tags: Vec<String>,
    #[serde(default)]
    pub link_sources: Vec<String>,
    #[serde(default)]
    pub link_hosts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub paused: bool,
    #[serde(default)]
    pub pause_revision: u64,
    #[serde(default = "default_ui_language")]
    pub ui_language: String,
    pub theme: String,
    pub layout: String,
    pub max_history: usize,
    pub shortcut: String,
    #[serde(default)]
    pub always_on_top: bool,
    #[serde(default)]
    pub retention_days: u32,
    #[serde(default = "default_show_startup")]
    pub show_startup: bool,
    #[serde(default = "default_sticker_pack")]
    pub sticker_pack: String,
}

/// Window behavior is saved independently from capture and retention settings.
/// Login startup registration belongs to the OS and is never cached here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowStartupPreferences {
    pub start_hidden: bool,
    pub show_tray_icon: bool,
}

impl Default for WindowStartupPreferences {
    fn default() -> Self {
        Self {
            start_hidden: false,
            show_tray_icon: true,
        }
    }
}

/// Preferences for the low-frequency stable update check. Kept separate from
/// ordinary settings so toggling updates never runs retention or changes
/// clipboard behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePreferences {
    pub revision: u64,
    pub auto_check: bool,
    pub skipped_version: Option<String>,
    pub last_check_at: Option<u64>,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            revision: 0,
            auto_check: true,
            skipped_version: None,
            last_check_at: None,
        }
    }
}
const AUTO_UPDATE_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

fn default_ui_language() -> String {
    "system".into()
}
fn default_show_startup() -> bool {
    true
}
fn default_sticker_pack() -> String {
    "auto".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            paused: true,
            pause_revision: 0,
            ui_language: default_ui_language(),
            theme: "dark".into(),
            layout: "grid".into(),
            max_history: 0,
            shortcut: "CommandOrControl+Shift+V".into(),
            always_on_top: false,
            retention_days: 0,
            show_startup: true,
            sticker_pack: default_sticker_pack(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !["dark", "light", "white", "forest", "rose", "system"].contains(&self.theme.as_str())
            || !["grid", "list", "horizontal"].contains(&self.layout.as_str())
            || (self.max_history != 0 && !(50..=10_000).contains(&self.max_history))
            || self.shortcut.trim().is_empty()
            || self.shortcut.len() > 80
        {
            return Err("设置无效：历史上限可选不限条数（0）或 50–10000 条。".into());
        }
        if !["system", "zh-CN", "en-US"].contains(&self.ui_language.as_str()) {
            return Err("不支持的界面语言。".into());
        }
        if !["auto", "none", "cosmos", "cats", "puppies", "garden"]
            .contains(&self.sticker_pack.as_str())
        {
            return Err("不支持的贴纸组合。".into());
        }
        if ![0, 1, 7, 30, 90, 180, 365].contains(&self.retention_days) {
            return Err("保留天数仅支持不限时间、1、7、30、90、180 或 365 天。".into());
        }
        Ok(())
    }
}

pub const STARPET_MOODS: &[&str] = &[
    "idle",
    "greet",
    "curious",
    "listening",
    "sleepy",
    "sleeping",
    "waking",
    "stretch",
    "happy",
    "delighted",
    "love",
    "shy",
    "surprised",
    "puzzled",
    "sad",
    "crying",
    "angry",
    "nervous",
    "proud",
    "relieved",
    "thinking",
    "searching",
    "reading",
    "writing",
    "drawing",
    "focused",
    "waiting",
    "loading",
    "success",
    "error",
    "celebrate",
    "resting",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct StarpetPreferences {
    pub enabled: bool,
    pub mood: String,
    pub color: String,
    pub size: u16,
}
impl Default for StarpetPreferences {
    fn default() -> Self {
        Self {
            enabled: false,
            mood: "idle".into(),
            color: "#FFD66E".into(),
            size: 100,
        }
    }
}
impl StarpetPreferences {
    pub fn validate(&self) -> Result<()> {
        if !STARPET_MOODS.contains(&self.mood.as_str()) {
            return Err("不支持的星伴表情。".into());
        }
        if self.color.len() != 7
            || !self.color.starts_with('#')
            || !self.color[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("星伴颜色需为六位 HEX，例如 #FFD66E。".into());
        }
        if !(70..=160).contains(&self.size) {
            return Err("星伴大小需为 70%–160%。".into());
        }
        Ok(())
    }

    pub fn window_size(&self) -> (f64, f64) {
        let body = 180.0 * f64::from(self.size) / 100.0;
        (body + 50.0, body + 80.0)
    }
}

#[cfg(any(feature = "desktop", test))]
pub(crate) fn starpet_resize_position(
    position: (f64, f64),
    previous_size: (f64, f64),
    next_size: (f64, f64),
    work_area: (f64, f64, f64, f64),
) -> (f64, f64) {
    let (left, top, width, height) = work_area;
    let x = position.0 + (previous_size.0 - next_size.0) / 2.0;
    let y = position.1 + previous_size.1 - next_size.1;
    (
        x.clamp(left, (left + width - next_size.0).max(left)),
        y.clamp(top, (top + height - next_size.1).max(top)),
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StarpetPreferencePatch {
    pub enabled: Option<bool>,
    pub mood: Option<String>,
    pub color: Option<String>,
    pub size: Option<u16>,
}
impl StarpetPreferencePatch {
    pub fn apply_to(self, preferences: &mut StarpetPreferences) {
        if let Some(enabled) = self.enabled {
            preferences.enabled = enabled;
        }
        if let Some(mood) = self.mood {
            preferences.mood = mood;
        }
        if let Some(color) = self.color {
            preferences.color = color.to_ascii_uppercase();
        }
        if let Some(size) = self.size {
            preferences.size = size;
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupFilter {
    pub min_bytes: u64,
    pub older_than_days: u32,
    pub kind: String,
    pub include_grouped: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub id: i64,
    pub title: String,
    pub kind: String,
    pub bytes: u64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPreview {
    pub token: String,
    pub total_count: usize,
    pub protected_count: usize,
    pub matched_count: usize,
    pub content_bytes: u64,
    pub database_bytes: u64,
    pub reusable_bytes: u64,
    pub items: Vec<CleanupItem>,
    pub truncated: bool,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub deleted_count: usize,
    pub content_bytes: u64,
    pub database_bytes_before: u64,
    pub database_bytes_after: u64,
    pub released_bytes: u64,
    pub warning: Option<String>,
}
pub struct CleanupSnapshot {
    pub preview: CleanupPreview,
    candidates: Vec<CleanupCandidate>,
}
struct CleanupCandidate {
    id: i64,
    fingerprint: [u8; 32],
}
pub const STALE_CLEANUP: &str = "清理预览已过期，请重新预览后确认。";
static CLEANUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
fn clip_bytes(clip: &Clip) -> u64 {
    clip.image
        .as_ref()
        .map_or(clip.content.len() as u64, |image| image.bytes)
        + clip.title.len() as u64
}
fn cleanup_fingerprint(clip: &Clip) -> Result<[u8; 32]> {
    // Keep the same stale-plan fields without retaining every candidate's body.
    // Copy count and privacy-only changes do not change the deletion target.
    let fields = (
        &clip.id,
        &clip.content,
        &clip.title,
        &clip.kind,
        &clip.language,
        &clip.image,
        &clip.updated_at,
        &clip.created_at,
        &clip.group_id,
        &clip.pinned,
        &clip.keep_forever,
    );
    Ok(Sha256::digest(serde_json::to_vec(&fields).map_err(|e| e.to_string())?).into())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Backup {
    format: String,
    version: u32,
    exported_at: i64,
    clips: Vec<Clip>,
    groups: Vec<Group>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<BackupImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    privacy_rules: Option<PrivacyRules>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BackupImage {
    clip_id: i64,
    png: String,
}

const CLIP_SELECT: &str = "SELECT c.id,c.content,c.title,c.kind,c.pinned,c.group_id,c.created_at,c.updated_at,c.copy_count,c.language,c.keep_forever,i.width,i.height,i.bytes,p.metadata FROM clips c LEFT JOIN clip_images i ON i.clip_id=c.id LEFT JOIN clip_privacy p ON p.clip_id=c.id";

pub struct Store {
    conn: Connection,
}

fn sql_error(error: rusqlite::Error) -> String {
    format!("本地数据库操作失败：{error}")
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
fn clip_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Clip> {
    let mut metadata = metadata_from_json(row.get(14)?)?;
    let kind: String = row.get(3)?;
    metadata.normalize_image_policy(&kind);
    Ok(Clip {
        record_identity: metadata.record_identity.clone(),
        id: row.get(0)?,
        content: row.get(1)?,
        title: row.get(2)?,
        kind,
        pinned: row.get(4)?,
        group_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        copy_count: row.get(8)?,
        language: row.get(9)?,
        image: match row.get::<_, Option<u32>>(11)? {
            Some(width) => Some(ImageMetadata {
                width,
                height: row.get(12)?,
                bytes: row.get(13)?,
            }),
            None => None,
        },
        entries: None,
        keep_forever: row.get(10)?,
        privacy_manual: metadata.privacy_manual,
        is_private: metadata.is_private(),
        tags: metadata.tags(),
        manual_tags: metadata.manual_tags.clone(),
        privacy_reasons: metadata.privacy_reasons,
        scan_version: metadata.scan_version,
        link_sources: metadata.link_sources,
        link_hosts: metadata.link_hosts,
    })
}

fn metadata_from_json(json: Option<String>) -> rusqlite::Result<Metadata> {
    match json {
        Some(json) => serde_json::from_str(&json).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(14, rusqlite::types::Type::Text, Box::new(e))
        }),
        None => Ok(Metadata::default()),
    }
}
fn read_metadata(conn: &Connection, id: i64) -> Result<Metadata> {
    let saved: Option<(String, String)> = conn
        .query_row(
            "SELECT p.metadata,c.kind FROM clip_privacy p JOIN clips c ON c.id=p.clip_id WHERE p.clip_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(sql_error)?;
    match saved {
        Some((json, kind)) => {
            let mut metadata = metadata_from_json(Some(json)).map_err(sql_error)?;
            metadata.normalize_image_policy(&kind);
            Ok(metadata)
        }
        None => Ok(Metadata::default()),
    }
}
fn put_metadata(conn: &Connection, id: i64, metadata: &Metadata) -> Result<()> {
    validate_backup_tags(&metadata.tags())?;
    conn.execute("INSERT INTO clip_privacy(clip_id,metadata) VALUES(?1,?2) ON CONFLICT(clip_id) DO UPDATE SET metadata=excluded.metadata", params![id, serde_json::to_string(metadata).map_err(|e| e.to_string())?]).map_err(sql_error)?;
    Ok(())
}
fn validate_backup_tags(tags: &[String]) -> Result<()> {
    if tags.len() > 256 {
        return Err("标签合并后超过 256 个，未写入。".into());
    }
    for chunk in tags.chunks(32) {
        privacy::validate_tags(chunk)?;
    }
    Ok(())
}
fn read_privacy_rules(conn: &Connection) -> Result<PrivacyRules> {
    let json: Option<String> = conn
        .query_row(
            "SELECT value FROM preferences WHERE key='privacy_rules_v1'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    let rules: PrivacyRules = match json {
        Some(json) => serde_json::from_str(&json).map_err(|_| "隐私规则损坏，已停止开放数据。")?,
        None => PrivacyRules::default(),
    };
    rules.validate()?;
    Ok(rules)
}
fn scan_clip(conn: &Connection, id: i64, rules: &PrivacyRules, reviewed: bool) -> Result<Metadata> {
    let (content, title, kind): (String, String, String) = conn
        .query_row(
            "SELECT content,title,kind FROM clips WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(sql_error)?;
    let mut metadata = read_metadata(conn, id)?.scanned(&content, &title, &kind, rules, reviewed);
    metadata.ensure_identity()?;
    put_metadata(conn, id, &metadata)?;
    Ok(metadata)
}

pub fn validate_content(content: &str, title: &str) -> Result<()> {
    if content.trim().is_empty() {
        return Err("内容不能为空。".into());
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err("单条内容不能超过 256 KB。".into());
    }
    if title.chars().count() > 200 {
        return Err("标题不能超过 200 个字符。".into());
    }
    Ok(())
}

/// Re-validates a backed-up file list without touching the disk, so restoring
/// on another machine never depends on the original paths still existing.
fn validate_backup_entries(entries: &[FileEntry]) -> Result<String> {
    if entries.is_empty() || entries.len() > crate::clipboard_files::MAX_ITEMS {
        return Err("备份文件记录数量无效。".into());
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut bytes = 0usize;
    for entry in entries {
        let path = entry.path.as_str();
        if path.is_empty()
            || path.contains('\0')
            || path.contains('\n')
            || path.contains('\r')
            || !Path::new(path).is_absolute()
            || !seen.insert(path)
        {
            return Err("备份文件路径无效。".into());
        }
        bytes = bytes.saturating_add(path.len() + 1);
    }
    if bytes > crate::clipboard_files::MAX_BYTES {
        return Err("备份文件列表超过 256 KiB。".into());
    }
    Ok(crate::clipboard_files::content(entries))
}

fn validate_group(name: &str, color: &str) -> Result<()> {
    if name.trim().is_empty() || name.chars().count() > 40 {
        return Err("分组名需为 1–40 个字符。".into());
    }
    if color.len() != 7
        || !color.starts_with('#')
        || !color[1..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err("分组颜色需为 #RRGGBB。".into());
    }
    Ok(())
}

const LANGUAGES: &[&str] = &[
    "plaintext",
    "javascript",
    "typescript",
    "python",
    "rust",
    "json",
    "html",
    "css",
    "bash",
    "sql",
    "markdown",
    "go",
    "java",
];
fn validate_language(language: &str) -> Result<()> {
    if !LANGUAGES.contains(&language) {
        return Err("不支持的代码语言。".into());
    }
    Ok(())
}

pub fn classify(content: &str) -> &'static str {
    let text = content.trim();
    if [4, 7, 9].contains(&text.len())
        && text.starts_with('#')
        && text[1..].bytes().all(|c| c.is_ascii_hexdigit())
    {
        "color"
    } else if (text.starts_with("https://") || text.starts_with("http://"))
        && !text.chars().any(char::is_whitespace)
    {
        "link"
    } else if text.starts_with("```")
        || (text.contains('\n')
            && [
                "const ",
                "function ",
                "fn ",
                "def ",
                "import ",
                "SELECT ",
                "<div",
                "{\n",
            ]
            .iter()
            .any(|s| text.contains(s)))
    {
        "code"
    } else {
        "text"
    }
}

impl Store {
    /// Read-only validation must never initialize, migrate, or prune a selected database.
    pub fn validate_database(path: &Path) -> Result<()> {
        if !path.is_file() {
            return Err("所选目录缺少 starclip.sqlite3，未创建新数据库。".into());
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sql_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(sql_error)?;
        if Self::check_schema_version(&conn)? == SCHEMA_VERSION {
            Self::validate_current_schema(&conn)?;
        }
        let tx = conn.unchecked_transaction().map_err(sql_error)?;
        let integrity: String = tx
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(sql_error)?;
        if integrity != "ok" {
            return Err("数据库完整性检查失败，未切换数据目录。".into());
        }
        // The original schema had no language column; normal startup upgrades it later.
        for query in [
            "SELECT id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count FROM clips LIMIT 0",
            "SELECT id,name,color FROM groups LIMIT 0",
            "SELECT key,value FROM preferences LIMIT 0",
        ] {
            tx.prepare(query).map_err(|_| "所选文件不是兼容的星笺数据库。".to_string())?;
        }
        if tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(sql_error)?
            .exists([])
            .map_err(sql_error)?
        {
            return Err("数据库分组引用损坏，未切换数据目录。".into());
        }
        let json: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some(json) = json {
            let settings: Settings =
                serde_json::from_str(&json).map_err(|_| "目标数据库的偏好设置损坏。")?;
            settings.validate()?;
        }
        Ok(())
    }

    /// SQLite includes committed WAL pages in this consistent standalone snapshot.
    pub fn snapshot(&self, target: &Path) -> Result<()> {
        let target = target.to_str().ok_or("目标路径不是有效的 Unicode 路径。")?;
        self.conn
            .execute("VACUUM main INTO ?1", [target])
            .map_err(sql_error)?;
        Ok(())
    }

    fn validate_current_schema(conn: &Connection) -> Result<()> {
        for query in [
            "SELECT id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count,language,keep_forever FROM clips LIMIT 0",
            "SELECT id,name,color FROM groups LIMIT 0",
            "SELECT key,value FROM preferences LIMIT 0",
            "SELECT clip_id,width,height,bytes,png,thumbnail FROM clip_images LIMIT 0",
            "SELECT clip_id,position,path,entry_kind FROM clip_files LIMIT 0",
            "SELECT clip_id,metadata FROM clip_privacy LIMIT 0",
        ] {
            conn.prepare(query).map_err(|_| "数据库结构损坏，请从恢复点恢复。".to_string())?;
        }
        Ok(())
    }

    pub(crate) fn check_schema_version(conn: &Connection) -> Result<u32> {
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(sql_error)?;
        if version > SCHEMA_VERSION {
            return Err(format!("此数据由较新版本的星笺创建（结构版本 {version}，当前支持 {SCHEMA_VERSION}）。请安装较新版本；原数据未更改。"));
        }
        Ok(version)
    }

    pub fn schema_version(&self) -> Result<u32> {
        Self::check_schema_version(&self.conn)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let directory = path.parent().ok_or("数据库路径缺少父目录。")?;
        Self::open_with_recovery(path, directory, &directory.join("recovery"))
    }

    pub(crate) fn open_with_recovery(
        path: &Path,
        selection: &Path,
        recovery: &Path,
    ) -> Result<Self> {
        let mut snapshot = None;
        if path.exists() {
            if fs::symlink_metadata(path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("数据库不能是符号链接。".into());
            }
            // Inspect read-only before any writable connection or migration is opened.
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(sql_error)?;
            conn.busy_timeout(std::time::Duration::from_secs(3))
                .map_err(sql_error)?;
            let version = Self::check_schema_version(&conn)?;
            let existing: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clips')", [], |row| row.get(0)).map_err(sql_error)?;
            if existing && version < SCHEMA_VERSION {
                snapshot = Some(crate::storage::create_recovery_snapshot(
                    recovery,
                    path.parent().ok_or("数据库路径无效。")?,
                    selection,
                    version,
                    "before-migration",
                    |target| {
                        conn.execute(
                            "VACUUM main INTO ?1",
                            [target.to_str().ok_or("备份路径无效。")?],
                        )
                        .map_err(sql_error)?;
                        Ok(())
                    },
                )?);
            }
        }
        let result = Connection::open(path)
            .map_err(sql_error)
            .and_then(Self::from_connection);
        result.map_err(|error| match snapshot {
            Some(point) => format!("数据库升级失败：{error}\n升级前恢复点已保留：{}。可在启动恢复中选择该恢复点；不会覆盖原目录。", point.path),
            None => error,
        })
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // SQLite lower() folds ASCII only; keep Unicode search consistent with the UI.
        conn.create_scalar_function(
            "starclip_contains",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(context
                    .get::<String>(0)?
                    .to_lowercase()
                    .contains(&context.get::<String>(1)?))
            },
        )
        .map_err(sql_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(sql_error)?;
        let version = Self::check_schema_version(&conn)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")
            .map_err(sql_error)?;
        if version == SCHEMA_VERSION {
            Self::validate_current_schema(&conn)?;
            let store = Self { conn };
            store.prune_history()?;
            return Ok(store);
        }
        // Every schema change and its version marker commit together. An error rolls
        // back even early ADD COLUMN steps, preserving the pre-upgrade database.
        let tx = conn.unchecked_transaction().map_err(sql_error)?;
        let existing_database: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clips')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS groups (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, color TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS clips (
                id INTEGER PRIMARY KEY, content TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, keep_forever INTEGER NOT NULL DEFAULT 0,
                group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, copy_count INTEGER NOT NULL DEFAULT 0, language TEXT
            );
            CREATE INDEX IF NOT EXISTS clips_updated ON clips(updated_at DESC);
            CREATE TABLE IF NOT EXISTS preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);").map_err(sql_error)?;
        if !existing_database {
            tx.execute("INSERT INTO preferences(key,value) VALUES('onboarding_v1','false') ON CONFLICT(key) DO NOTHING", []).map_err(sql_error)?;
        }
        let columns = {
            let mut stmt = tx.prepare("PRAGMA table_info(clips)").map_err(sql_error)?;
            let columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(sql_error)?;
            columns
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?
        };
        if !columns.iter().any(|name| name == "language") {
            tx.execute("ALTER TABLE clips ADD COLUMN language TEXT", [])
                .map_err(sql_error)?;
        }
        if !columns.iter().any(|name| name == "keep_forever") {
            tx.execute(
                "ALTER TABLE clips ADD COLUMN keep_forever INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(sql_error)?;
        }
        // The original UNIQUE(content) mixed text and image identities. Rebuild once,
        // in one transaction, before any image table exists; preserve every original field.
        let legacy_unique: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_index_list('clips') WHERE origin='u')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if legacy_unique {
            let has_images: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clip_images')", [], |row| row.get(0)).map_err(sql_error)?;
            if has_images {
                return Err("图片数据库结构不兼容，原数据未更改。".into());
            }
            tx.execute_batch("CREATE TABLE clips_images_upgrade (
                id INTEGER PRIMARY KEY, content TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, keep_forever INTEGER NOT NULL DEFAULT 0,
                group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, copy_count INTEGER NOT NULL DEFAULT 0, language TEXT);
                INSERT INTO clips_images_upgrade(id,content,title,kind,pinned,keep_forever,group_id,created_at,updated_at,copy_count,language)
                    SELECT id,content,title,kind,pinned,keep_forever,group_id,created_at,updated_at,copy_count,language FROM clips;
                DROP TABLE clips;
                ALTER TABLE clips_images_upgrade RENAME TO clips;").map_err(sql_error)?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS clips_updated ON clips(updated_at DESC);
            CREATE TABLE IF NOT EXISTS clip_images (
                clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,
                width INTEGER NOT NULL CHECK(width>0), height INTEGER NOT NULL CHECK(height>0),
                bytes INTEGER NOT NULL CHECK(bytes>0), png BLOB NOT NULL, thumbnail BLOB NOT NULL
            );",
        )
        .map_err(sql_error)?;
        // File records need their own identity space: the text index must stop
        // covering them, otherwise a file list would collide with pasted text
        // that happens to read like a path. Dropping only relaxes the predicate.
        tx.execute_batch("DROP INDEX IF EXISTS clips_text_content;
            CREATE UNIQUE INDEX IF NOT EXISTS clips_text_content ON clips(content) WHERE kind NOT IN ('image','file');
            CREATE UNIQUE INDEX IF NOT EXISTS clips_image_content ON clips(content) WHERE kind='image';
            CREATE UNIQUE INDEX IF NOT EXISTS clips_file_content ON clips(content) WHERE kind='file';
            CREATE TABLE IF NOT EXISTS clip_files (
                clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                position INTEGER NOT NULL CHECK(position>=0),
                path TEXT NOT NULL,
                entry_kind TEXT NOT NULL CHECK(entry_kind IN ('file','directory','unknown')),
                PRIMARY KEY(clip_id,position)
            );
            CREATE UNIQUE INDEX IF NOT EXISTS clip_files_path ON clip_files(clip_id,path);")
            .map_err(sql_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS clip_privacy (clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE, metadata TEXT NOT NULL);").map_err(sql_error)?;
        let rules = read_privacy_rules(&tx)?;
        let ids = {
            let mut stmt = tx.prepare("SELECT id FROM clips").map_err(sql_error)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(sql_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?
        };
        for id in ids {
            scan_clip(&tx, id, &rules, false)?;
        }
        Self::validate_current_schema(&tx)?;
        if tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(sql_error)?
            .exists([])
            .map_err(sql_error)?
        {
            return Err("数据库引用损坏，升级已撤销。".into());
        }
        let json: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let settings: Settings = match json {
            Some(json) => serde_json::from_str(&json).map_err(|_| "偏好设置损坏，升级已撤销。")?,
            None => Settings::default(),
        };
        settings.validate()?;
        Self::prune_with_settings(&tx, &settings, now())?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(Self { conn })
    }

    pub fn settings(&self) -> Result<Settings> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                let settings: Settings =
                    serde_json::from_str(&json).map_err(|_| "偏好设置损坏，请从备份恢复。")?;
                settings.validate()?;
                Ok(settings)
            }
            None => Ok(Settings::default()),
        }
    }

    pub fn window_startup_preferences(&self) -> Result<WindowStartupPreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='window_startup_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => serde_json::from_str(&json)
                .map_err(|_| "启动与菜单栏偏好损坏，请从备份恢复。".into()),
            None => Ok(WindowStartupPreferences::default()),
        }
    }

    pub fn save_window_startup_preferences(
        &self,
        preferences: &WindowStartupPreferences,
    ) -> Result<()> {
        let json = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('window_startup_v1',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn telemetry_preferences(&self) -> Result<TelemetryPreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='telemetry_preferences_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                let preferences: TelemetryPreferences = serde_json::from_str(&json)
                    .map_err(|_| "统计偏好损坏，请重新设置。".to_string())?;
                if preferences.revision > MAX_REVISION {
                    return Err("统计偏好损坏，请重新设置。".into());
                }
                Ok(preferences)
            }
            None => Ok(TelemetryPreferences::default()),
        }
    }

    pub fn set_telemetry_mode(&self, mode: TelemetryMode) -> Result<TelemetryPreferences> {
        // Only an explicit local choice can replace damaged preferences. This
        // never touches ordinary settings or retention.
        let mut preferences = TelemetryPreferences {
            revision: self
                .telemetry_preferences()
                .map(|value| value.revision)
                .unwrap_or(0),
            mode,
        };
        preferences.revision = preferences
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or("统计偏好修订已达上限。")?;
        let json = serde_json::to_string(&preferences).map_err(|error| error.to_string())?;
        self.conn.execute(
            "INSERT INTO preferences(key,value) VALUES('telemetry_preferences_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [json],
        ).map_err(sql_error)?;
        Ok(preferences)
    }

    pub fn update_preferences(&self) -> Result<UpdatePreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='update_preferences_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                serde_json::from_str(&json).map_err(|_| "更新偏好损坏，请重新启动星笺。".into())
            }
            None => Ok(UpdatePreferences::default()),
        }
    }

    fn save_update_preferences(&self, preferences: &mut UpdatePreferences) -> Result<()> {
        preferences.revision = preferences
            .revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_REVISION)
            .ok_or("更新偏好修订已达上限。")?;
        let json = serde_json::to_string(preferences).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('update_preferences_v1',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn set_auto_update_check(&self, enabled: bool) -> Result<UpdatePreferences> {
        let mut preferences = self.update_preferences()?;
        if preferences.auto_check != enabled {
            preferences.auto_check = enabled;
            self.save_update_preferences(&mut preferences)?;
        }
        Ok(preferences)
    }

    pub fn skip_update_version(&self, version: &str) -> Result<UpdatePreferences> {
        let mut preferences = self.update_preferences()?;
        preferences.skipped_version = Some(version.to_owned());
        self.save_update_preferences(&mut preferences)?;
        Ok(preferences)
    }

    // The caller holds the storage/store/updater locks in that order. Persist
    // before requesting; both a failed request and a restart consume the interval.
    pub fn record_update_check(
        &self,
        now: u64,
        automatic: bool,
    ) -> Result<(UpdatePreferences, bool)> {
        let mut preferences = self.update_preferences()?;
        if automatic {
            if !preferences.auto_check {
                return Ok((preferences, false));
            }
            if let Some(last) = preferences.last_check_at {
                if now < last {
                    preferences.last_check_at = Some(now);
                    self.save_update_preferences(&mut preferences)?;
                    return Ok((preferences, false));
                }
                if now - last < AUTO_UPDATE_INTERVAL_MS {
                    return Ok((preferences, false));
                }
            }
        }
        preferences.last_check_at = Some(now);
        self.save_update_preferences(&mut preferences)?;
        Ok((preferences, true))
    }

    pub fn onboarding_completed(&self) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT COALESCE((SELECT value='true' FROM preferences WHERE key='onboarding_v1'), 1)",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }

    pub fn complete_onboarding(&self) -> Result<()> {
        // This marker must not run save_settings, pruning, or clipboard transitions.
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('onboarding_v1','true')
            ON CONFLICT(key) DO UPDATE SET value='true'",
                [],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn appearance(&self) -> Result<Appearance> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='appearance_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let value = match json {
            Some(json) => serde_json::from_str::<Appearance>(&json)
                .map_err(|_| "自定义外观数据损坏，未修改已保存的数据。")?,
            None => Appearance::default(),
        };
        value
            .validate()
            .map_err(|_| "自定义外观数据损坏，未修改已保存的数据。")?;
        Ok(value)
    }

    fn change_appearance(
        &self,
        change: impl FnOnce(&Appearance) -> Result<Appearance>,
    ) -> Result<Appearance> {
        // Appearance updates never save ordinary settings, run retention or award growth.
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let old = self.appearance()?;
        let next = change(&old)?;
        next.validate()?;
        if next != old {
            let json = serde_json::to_string(&next).map_err(|_| "无法保存自定义外观。")?;
            tx.execute("INSERT INTO preferences(key,value) VALUES('appearance_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(next)
    }

    pub fn save_appearance(&self, patch: &serde_json::Value) -> Result<Appearance> {
        self.change_appearance(|value| value.patched(patch))
    }

    pub fn import_appearance_sticker(&self, png_base64: &str, name: &str) -> Result<Appearance> {
        let sticker = appearance::prepare_sticker(png_base64, name)?;
        self.change_appearance(|value| Ok(value.with_sticker(sticker)))
    }

    pub fn remove_appearance_sticker(&self) -> Result<Appearance> {
        self.change_appearance(|value| Ok(value.without_sticker()))
    }

    pub fn starpet_preferences(&self) -> Result<StarpetPreferences> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='starpet_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        match json {
            Some(json) => {
                let mut preferences: StarpetPreferences = serde_json::from_str(&json)
                    .map_err(|_| "星伴偏好损坏，未更改已保存的数据。")?;
                preferences.validate()?;
                preferences.color.make_ascii_uppercase();
                Ok(preferences)
            }
            None => Ok(StarpetPreferences::default()),
        }
    }

    pub fn starpet_growth(&self) -> Result<GrowthState> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='starpet_growth_v1'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let state = match json {
            Some(json) => serde_json::from_str::<GrowthState>(&json)
                .map_err(|_| "星伴成长存档损坏，未修改已保存的数据。")?,
            None => GrowthState::default(),
        };
        state.validate()?;
        Ok(state)
    }

    pub fn change_starpet_growth(&self, change: GrowthChange<'_>) -> Result<GrowthState> {
        let today: String = self
            .conn
            .query_row("SELECT strftime('%Y-%m-%d','now','localtime')", [], |row| {
                row.get(0)
            })
            .map_err(sql_error)?;
        self.change_starpet_growth_on(change, &today)
    }

    fn change_starpet_growth_on(
        &self,
        change: GrowthChange<'_>,
        today: &str,
    ) -> Result<GrowthState> {
        // One transaction contains read, validation and write; unrelated preferences and clips are untouched.
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let old = self.starpet_growth()?;
        let mut state = old.clone();
        change.apply(&mut state, today)?;
        if state != old {
            state.revision = old
                .revision
                .checked_add(1)
                .filter(|r| *r <= MAX_REVISION)
                .ok_or("成长存档修订次数超限，未修改数据。")?;
            let json = serde_json::to_string(&state).map_err(|_| "无法保存星伴成长数据。")?;
            tx.execute("INSERT INTO preferences(key,value) VALUES('starpet_growth_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(sql_error)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(state)
    }

    pub fn save_starpet_preferences(&self, preferences: &StarpetPreferences) -> Result<()> {
        // Keep pet interactions independent of retention, clipboard capture and ordinary settings.
        preferences.validate()?;
        let mut preferences = preferences.clone();
        preferences.color.make_ascii_uppercase();
        let json = serde_json::to_string(&preferences).map_err(|e| e.to_string())?;
        self.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('starpet_v1',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [json],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn set_theme(&self, theme: &str) -> Result<Settings> {
        if !["dark", "light", "white", "forest", "rose", "system"].contains(&theme) {
            return Err("不支持的应用主题。".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let mut json: serde_json::Value = match stored {
            Some(json) => serde_json::from_str(&json).map_err(|_| "偏好设置损坏，主题未保存。")?,
            None => serde_json::to_value(Settings::default()).map_err(|e| e.to_string())?,
        };
        json.as_object_mut()
            .ok_or("偏好设置损坏，主题未保存。")?
            .insert("theme".into(), theme.into());
        let settings: Settings =
            serde_json::from_value(json.clone()).map_err(|_| "偏好设置损坏，主题未保存。")?;
        settings.validate()?;
        // Preserve every other stored preference, including older missing fields and future keys.
        tx.execute("INSERT INTO preferences(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json.to_string()]).map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.save_settings_with_pause_guard(settings, None)
            .map(|_| ())
    }

    pub fn save_settings_with_pause_guard(
        &self,
        settings: &Settings,
        expected_pause_revision: Option<u64>,
    ) -> Result<Settings> {
        settings.validate()?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let mut settings = settings.clone();
        // Read and write on this transaction. The version detects even an
        // unseen pause -> resume -> pause round trip while a draft is open.
        let current = self.settings()?;
        if expected_pause_revision.is_some_and(|expected| expected != current.pause_revision) {
            settings.paused = current.paused;
        }
        // The store owns this version; never persist a caller-supplied value.
        settings.pause_revision = if settings.paused != current.paused {
            current
                .pause_revision
                .checked_add(1)
                .ok_or("采集设置版本已达上限，偏好设置未保存。")?
        } else {
            current.pause_revision
        };
        let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO preferences(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]).map_err(sql_error)?;
        Self::prune_with_settings(&tx, &settings, now())?;
        tx.commit().map_err(sql_error)?;
        Ok(settings)
    }

    pub fn privacy_rules(&self) -> Result<PrivacyRules> {
        read_privacy_rules(&self.conn)
    }

    pub fn save_privacy_rules(&self, requested: &PrivacyRules) -> Result<PrivacyRules> {
        let mut requested = requested.clone();
        for rule in &mut requested.rules {
            rule.tag = rule
                .tag
                .as_ref()
                .map(|tag| tag.trim().to_owned())
                .filter(|tag| !tag.is_empty());
        }
        requested.validate()?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let current = self.privacy_rules()?;
        if requested.revision != current.revision {
            return Err("规则已变化，请重新打开后再保存。".into());
        }
        let mut next = requested.clone();
        if current.rules == next.rules {
            return Ok(current);
        }
        next.revision = current
            .revision
            .checked_add(1)
            .ok_or("规则版本达到上限。")?;
        tx.execute("INSERT INTO preferences(key,value) VALUES('privacy_rules_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&next).map_err(|e| e.to_string())?]).map_err(sql_error)?;
        // Older scans immediately become ineligible for AI; only a completed new scan restores eligibility.
        tx.commit().map_err(sql_error)?;
        Ok(next)
    }

    pub fn set_clip_privacy(&self, id: i64, private: bool, reviewed: bool) -> Result<Clip> {
        if !private && !reviewed {
            return Err("解除隐私前需要在本机确认已经复核内容。".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        self.clip(id)?;
        let mut metadata = read_metadata(&tx, id)?;
        metadata.privacy_manual = private;
        put_metadata(&tx, id, &metadata)?;
        // A local review may remove stale reasons, never an active match.
        scan_clip(&tx, id, &self.privacy_rules()?, !private && reviewed)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    pub fn set_clip_tags(&self, id: i64, tags: &[String]) -> Result<Clip> {
        privacy::validate_tags(tags)?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        self.clip(id)?;
        let mut metadata = read_metadata(&tx, id)?;
        metadata.manual_tags = privacy::union(tags, &[]);
        put_metadata(&tx, id, &metadata)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    fn privacy_rescan_plan(&self) -> Result<(PrivacyRescan, Vec<(i64, Metadata)>)> {
        let rules = self.privacy_rules()?;
        let mut statement = self
            .conn
            .prepare(&format!(
                "{CLIP_SELECT} ORDER BY c.updated_at DESC,c.id DESC"
            ))
            .map_err(sql_error)?;
        let clips = statement.query_map([], clip_from_row).map_err(sql_error)?;
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&rules).map_err(|e| e.to_string())?);
        let mut report = PrivacyRescan {
            token: String::new(),
            total: 0,
            private_count: 0,
            newly_private_count: 0,
            changed_count: 0,
        };
        let mut changes = Vec::new();
        for clip in clips {
            let clip = clip.map_err(sql_error)?;
            report.total += 1;
            let old = read_metadata(&self.conn, clip.id)?;
            digest.update(serde_json::to_vec(&clip).map_err(|e| e.to_string())?);
            digest.update(serde_json::to_vec(&old).map_err(|e| e.to_string())?);
            let mut next = old.scanned(&clip.content, &clip.title, &clip.kind, &rules, false);
            next.ensure_identity()?;
            report.private_count += usize::from(next.is_private());
            report.newly_private_count += usize::from(next.is_private() && !old.is_private());
            if old != next {
                report.changed_count += 1;
                changes.push((clip.id, next));
            }
        }
        report.token = format!("{:x}", digest.finalize());
        Ok((report, changes))
    }

    pub fn privacy_rescan_preview(&self) -> Result<PrivacyRescan> {
        self.privacy_rescan_plan().map(|(report, _)| report)
    }

    pub fn privacy_rescan_apply(&self, token: &str) -> Result<PrivacyRescan> {
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let (report, changes) = self.privacy_rescan_plan()?;
        if report.token != token {
            return Err("记录或规则已变化，请重新预览扫描结果。".into());
        }
        for (id, metadata) in changes {
            put_metadata(&tx, id, &metadata)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(report)
    }

    /// Read only one page of content. Counts/facets come from the whole database.
    pub fn history_page(&self, query: &HistoryQuery) -> Result<HistoryPage> {
        if query.query.len() > MAX_CONTENT_BYTES
            || query.sources.len() > 100
            || query.sources.iter().any(|source| source.len() > 512)
            || query.tag.len() > 160
        {
            return Err("历史筛选条件过长。".into());
        }
        let revision = self.privacy_rules()?.revision;
        let mut predicates = vec!["1=1".to_owned()];
        let mut values: Vec<rusqlite::types::Value> = vec![];
        match query.filter.as_str() {
            "all" => {}
            "pinned" => predicates.push("c.pinned=1".into()),
            "kept" => predicates.push("c.keep_forever=1".into()),
            "link" => predicates
                .push("(c.kind='link' OR json_array_length(p.metadata,'$.linkHosts')>0)".into()),
            "text" | "code" | "color" | "image" | "file" => {
                predicates.push("c.kind=?".into());
                values.push(query.filter.clone().into());
            }
            group if group.starts_with("group-") => {
                let id = group[6..].parse::<i64>().map_err(|_| "分组编号无效。")?;
                predicates.push("c.group_id=?".into());
                values.push(id.into());
            }
            _ => return Err("历史分类无效。".into()),
        }
        let search = query.query.trim().to_lowercase();
        if !search.is_empty() {
            let tags = if query.ai_eligible {
                " || COALESCE(json_extract(p.metadata,'$.manualTags'),'') || COALESCE(json_extract(p.metadata,'$.ruleTags'),'')"
            } else {
                ""
            };
            predicates.push(format!("starclip_contains(c.title || char(10) || CASE WHEN c.kind='image' THEN '' ELSE c.content END{tags},?)"));
            values.push(search.into());
        }
        if !query.sources.is_empty() {
            let mut sources = vec![];
            for source in &query.sources {
                let (path, value) = match source.strip_prefix("host:") {
                    Some(host) => ("linkHosts", host),
                    None => ("linkSources", source.as_str()),
                };
                sources.push(format!(
                    "EXISTS(SELECT 1 FROM json_each(p.metadata,'$.{path}') WHERE value=?)"
                ));
                values.push(value.to_owned().into());
            }
            predicates.push(format!("({})", sources.join(" OR ")));
        }
        if !query.tag.is_empty() {
            predicates.push("(EXISTS(SELECT 1 FROM json_each(p.metadata,'$.manualTags') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.ruleTags') WHERE value=?))".into());
            values.extend([query.tag.clone().into(), query.tag.clone().into()]);
        }
        // A legacy image capability marker is not a privacy match; image pixels
        // remain unscanned and can never become AI-eligible via this query.
        let private = "(COALESCE(json_extract(p.metadata,'$.privacyManual'),0)=1 OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.privacyReasons') WHERE c.kind!='image' OR value!='图片尚未进行文字识别'))";
        let pending = format!(
            "(c.kind='image' OR COALESCE(json_extract(p.metadata,'$.scanVersion'),0)!={revision})"
        );
        match query.privacy.as_str() {
            "all" => {}
            "private" => predicates.push(private.into()),
            "pending" => predicates.push(pending.clone()),
            "ordinary" => predicates.push(format!("NOT {private} AND NOT {pending}")),
            _ => return Err("隐私筛选无效。".into()),
        }
        if query.ai_eligible {
            // Creation features only accept text and recognized-image records.
            predicates.push(format!(
                "c.kind NOT IN ('image','file') AND NOT {private} AND NOT {pending}"
            ));
        }
        let from = "FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id";
        let where_sql = predicates.join(" AND ");
        let total: u64 = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) {from} WHERE {where_sql}"),
                rusqlite::params_from_iter(values.iter()),
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        let last = total
            .div_ceil(u64::from(HISTORY_PAGE_SIZE))
            .max(1)
            .min(u64::from(u32::MAX)) as u32;
        let page = query.page.max(1).min(last);
        let offset = u64::from(page - 1) * u64::from(HISTORY_PAGE_SIZE);
        let mut statement = self.conn.prepare(&format!("{CLIP_SELECT} WHERE {where_sql} ORDER BY c.updated_at DESC,c.id DESC LIMIT {HISTORY_PAGE_SIZE} OFFSET {offset}")).map_err(sql_error)?;
        let mut clips = statement
            .query_map(rusqlite::params_from_iter(values.iter()), clip_from_row)
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        Self::fill_entries(&self.conn, &mut clips)?;
        let mut counts = BTreeMap::new();
        let mut statement = self
            .conn
            .prepare("SELECT kind,COUNT(*) FROM clips GROUP BY kind")
            .map_err(sql_error)?;
        for item in statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
            })
            .map_err(sql_error)?
        {
            let (kind, count) = item.map_err(sql_error)?;
            counts.insert(kind, count);
        }
        counts.insert("all".into(), counts.values().sum());
        for (key, predicate) in [
            ("pinned", "c.pinned=1"),
            ("kept", "c.keep_forever=1"),
            (
                "link",
                "c.kind='link' OR json_array_length(p.metadata,'$.linkHosts')>0",
            ),
        ] {
            counts.insert(
                key.into(),
                self.conn
                    .query_row(
                        &format!("SELECT COUNT(*) {from} WHERE {predicate}"),
                        [],
                        |row| row.get(0),
                    )
                    .map_err(sql_error)?,
            );
        }
        let mut statement = self
            .conn
            .prepare(
                "SELECT group_id,COUNT(*) FROM clips WHERE group_id IS NOT NULL GROUP BY group_id",
            )
            .map_err(sql_error)?;
        let group_counts = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(sql_error)?
            .collect::<rusqlite::Result<BTreeMap<_, _>>>()
            .map_err(sql_error)?;
        let facets = |paths: &[&str]| -> Result<Vec<String>> {
            let sql = paths.iter().map(|path| format!("SELECT value FROM clip_privacy,json_each(metadata,'$.{path}') WHERE type='text'")).collect::<Vec<_>>().join(" UNION ");
            let mut statement = self
                .conn
                .prepare(&format!(
                    "SELECT DISTINCT value FROM ({sql}) ORDER BY value"
                ))
                .map_err(sql_error)?;
            let values = statement
                .query_map([], |row| row.get(0))
                .map_err(sql_error)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?;
            Ok(values)
        };
        Ok(HistoryPage {
            clips,
            history: HistorySummary {
                page,
                page_size: HISTORY_PAGE_SIZE,
                total,
                counts,
                group_counts,
                sources: facets(&["linkSources"])?,
                hosts: facets(&["linkHosts"])?,
                tags: facets(&["manualTags", "ruleTags"])?,
            },
        })
    }

    /// File references live in their own table, so pages never duplicate rows.
    fn fill_entries(conn: &Connection, clips: &mut [Clip]) -> Result<()> {
        let ids: Vec<i64> = clips
            .iter()
            .filter(|clip| clip.kind == "file")
            .map(|clip| clip.id)
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut statement = conn
            .prepare(&format!(
                "SELECT clip_id,path,entry_kind FROM clip_files WHERE clip_id IN ({placeholders}) ORDER BY clip_id,position"
            ))
            .map_err(sql_error)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(sql_error)?;
        let mut grouped: HashMap<i64, Vec<FileEntry>> = HashMap::new();
        for row in rows {
            let (id, path, kind) = row.map_err(sql_error)?;
            let entry_kind = match kind.as_str() {
                "file" => FileEntryKind::File,
                "directory" => FileEntryKind::Directory,
                _ => FileEntryKind::Unknown,
            };
            grouped
                .entry(id)
                .or_default()
                .push(FileEntry { path, entry_kind });
        }
        for clip in clips.iter_mut().filter(|clip| clip.kind == "file") {
            clip.entries = grouped.remove(&clip.id);
        }
        Ok(())
    }

    pub fn clips(&self) -> Result<Vec<Clip>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "{CLIP_SELECT} ORDER BY c.updated_at DESC,c.id DESC"
            ))
            .map_err(sql_error)?;
        let rows = stmt.query_map([], clip_from_row).map_err(sql_error)?;
        let mut clips = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        Self::fill_entries(&self.conn, &mut clips)?;
        Ok(clips)
    }

    pub fn groups(&self) -> Result<Vec<Group>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id,name,color FROM groups ORDER BY id")
            .map_err(sql_error)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Group {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                })
            })
            .map_err(sql_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)
    }

    /// Refresh explicitly selected details without loading the entire history.
    /// Missing IDs are omitted so callers can discard deleted selections.
    pub fn clip_details(&self, ids: &[i64]) -> Result<Vec<Clip>> {
        if ids.len() > 500 || ids.iter().any(|id| *id <= 0) {
            return Err("每次最多读取 500 条有效记录。".into());
        }
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut statement = self
            .conn
            .prepare(&format!(
                "{CLIP_SELECT} WHERE c.id IN ({placeholders}) ORDER BY c.id"
            ))
            .map_err(sql_error)?;
        let clips = statement
            .query_map(rusqlite::params_from_iter(ids.iter()), clip_from_row)
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        let mut clips = clips;
        Self::fill_entries(&self.conn, &mut clips)?;
        Ok(clips)
    }

    pub fn has_pending_ai_history(&self) -> Result<bool> {
        let revision = self.privacy_rules()?.revision;
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id WHERE c.kind!='image' AND COALESCE(json_extract(p.metadata,'$.scanVersion'),0) != ?1)", [revision], |row| row.get(0)).map_err(sql_error)
    }

    pub fn clip(&self, id: i64) -> Result<Clip> {
        let mut clip = self
            .conn
            .query_row(&format!("{CLIP_SELECT} WHERE c.id=?1"), [id], clip_from_row)
            .optional()
            .map_err(sql_error)?
            .ok_or_else(|| "这条星笺已被删除。".to_string())?;
        Self::fill_entries(&self.conn, std::slice::from_mut(&mut clip))?;
        Ok(clip)
    }

    pub fn add(&self, content: &str, title: &str, group_id: Option<i64>) -> Result<Clip> {
        self.add_protected(content, title, group_id, false)
    }

    fn put_image(conn: &Connection, id: i64, image: &PreparedImage) -> Result<()> {
        let metadata = image.metadata();
        conn.execute("INSERT INTO clip_images(clip_id,width,height,bytes,png,thumbnail) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(clip_id) DO NOTHING",
            params![id, metadata.width, metadata.height, metadata.bytes, image.png, image.thumbnail]).map_err(sql_error)?;
        Ok(())
    }

    pub fn add_image(
        &self,
        image: &PreparedImage,
        title: &str,
        group_id: Option<i64>,
        keep_forever: bool,
    ) -> Result<Clip> {
        validate_content(image.digest(), title)?;
        let timestamp = now();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        tx.execute("INSERT INTO clips(content,title,kind,group_id,created_at,updated_at,keep_forever) VALUES(?1,?2,'image',?3,?4,?4,?5)
            ON CONFLICT(content) WHERE kind='image' DO UPDATE SET updated_at=excluded.updated_at,
            title=CASE WHEN excluded.title<>'' THEN excluded.title ELSE clips.title END,
            group_id=COALESCE(excluded.group_id,clips.group_id),keep_forever=MAX(clips.keep_forever,excluded.keep_forever)",
            params![image.digest(), title.trim(), group_id, timestamp, keep_forever]).map_err(sql_error)?;
        let id: i64 = tx
            .query_row(
                "SELECT id FROM clips WHERE content=?1 AND kind='image'",
                [image.digest()],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        Self::put_image(&tx, id, image)?;
        scan_clip(&tx, id, &self.privacy_rules()?, false)?;
        Self::prune_with_settings(&tx, &self.settings()?, timestamp)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    pub fn image_png(&self, id: i64, thumbnail: bool) -> Result<Vec<u8>> {
        let column = if thumbnail { "thumbnail" } else { "png" };
        self.conn.query_row(&format!("SELECT i.{column} FROM clip_images i JOIN clips c ON c.id=i.clip_id WHERE c.id=?1 AND c.kind='image' AND length(i.{column})<=?2"),
            params![id, images::MAX_PNG_BYTES], |row| row.get(0)).optional().map_err(sql_error)?
            .ok_or_else(|| "图片不存在、已被删除或超过读取上限。".into())
    }

    pub fn add_protected(
        &self,
        content: &str,
        title: &str,
        group_id: Option<i64>,
        keep_forever: bool,
    ) -> Result<Clip> {
        validate_content(content, title)?;
        let timestamp = now();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        tx.execute("INSERT INTO clips(content,title,kind,group_id,created_at,updated_at,keep_forever) VALUES(?1,?2,?3,?4,?5,?5,?6)
            ON CONFLICT(content) WHERE kind NOT IN ('image','file') DO UPDATE SET updated_at=excluded.updated_at,
            title=CASE WHEN excluded.title<>'' THEN excluded.title ELSE clips.title END,
            group_id=COALESCE(excluded.group_id,clips.group_id),
            keep_forever=MAX(clips.keep_forever,excluded.keep_forever)", params![content, title.trim(), classify(content), group_id, timestamp, keep_forever]).map_err(sql_error)?;
        let id = tx
            .query_row(
                "SELECT id FROM clips WHERE content=?1 AND kind NOT IN ('image','file')",
                [content],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        scan_clip(&tx, id, &self.privacy_rules()?, false)?;
        Self::prune_with_settings(&tx, &self.settings()?, timestamp)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    /// Stores a native file list. Paths are only ever references: nothing is
    /// read from disk beyond metadata, and the original files are never touched.
    pub fn add_files(
        &self,
        entries: &[FileEntry],
        title: &str,
        group_id: Option<i64>,
        keep_forever: bool,
    ) -> Result<Clip> {
        if entries.is_empty() {
            return Err("没有可记录的文件。".into());
        }
        let content = crate::clipboard_files::content(entries);
        validate_content(&content, title)?;
        let timestamp = now();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        tx.execute("INSERT INTO clips(content,title,kind,group_id,created_at,updated_at,keep_forever) VALUES(?1,?2,'file',?3,?4,?4,?5)
            ON CONFLICT(content) WHERE kind='file' DO UPDATE SET updated_at=excluded.updated_at,
            title=CASE WHEN excluded.title<>'' THEN excluded.title ELSE clips.title END,
            group_id=COALESCE(excluded.group_id,clips.group_id),
            keep_forever=MAX(clips.keep_forever,excluded.keep_forever)",
            params![content, title.trim(), group_id, timestamp, keep_forever]).map_err(sql_error)?;
        let id: i64 = tx
            .query_row(
                "SELECT id FROM clips WHERE content=?1 AND kind='file'",
                [&content],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        // Re-copying the same paths keeps one record but refreshes entry kinds,
        // so a folder that became a file (or a vanished path) is not stale.
        tx.execute("DELETE FROM clip_files WHERE clip_id=?1", [id])
            .map_err(sql_error)?;
        {
            let mut statement = tx
                .prepare(
                    "INSERT INTO clip_files(clip_id,position,path,entry_kind) VALUES(?1,?2,?3,?4)",
                )
                .map_err(sql_error)?;
            for (position, entry) in entries.iter().enumerate() {
                statement
                    .execute(params![
                        id,
                        position as i64,
                        entry.path,
                        entry.entry_kind.as_str()
                    ])
                    .map_err(sql_error)?;
            }
        }
        scan_clip(&tx, id, &self.privacy_rules()?, false)?;
        Self::prune_with_settings(&tx, &self.settings()?, timestamp)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    pub fn save_code(
        &self,
        id: Option<i64>,
        title: &str,
        content: &str,
        language: &str,
        group_id: Option<i64>,
    ) -> Result<Clip> {
        validate_content(content, title)?;
        validate_language(language)?;
        let timestamp = now();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let clip = match id {
            Some(id) => {
                if self.clip(id)?.kind == "image" {
                    return Err("图片不能转换为代码，请新建代码片段。".into());
                }
                if self.clip(id)?.kind == "file" {
                    return Err("文件记录不能转换为代码，请新建代码片段。".into());
                }
                self.conn.execute("UPDATE clips SET content=?1,title=?2,kind='code',language=?3,group_id=?4,updated_at=?5,pinned=1 WHERE id=?6", params![content,title.trim(),language,group_id,timestamp,id])
                    .map_err(|error| match error {
                        rusqlite::Error::SqliteFailure(ref details, _) if details.extended_code == 2067 => "已存在相同内容，请编辑已有星笺。".into(),
                        other => sql_error(other),
                    })?;
                self.clip(id)
            }
            None => {
                self.conn.execute("INSERT INTO clips(content,title,kind,language,group_id,pinned,created_at,updated_at) VALUES(?1,?2,'code',?3,?4,1,?5,?5)", params![content,title.trim(),language,group_id,timestamp])
                    .map_err(|error| match error {
                        rusqlite::Error::SqliteFailure(ref details, _) if details.extended_code == 2067 => "已存在相同内容，请编辑已有星笺。".into(),
                        other => sql_error(other),
                    })?;
                self.clip(self.conn.last_insert_rowid())
            }
        }?;
        scan_clip(&tx, clip.id, &self.privacy_rules()?, false)?;
        self.prune_history()?;
        let clip = self.clip(clip.id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    pub fn update(&self, id: i64, title: &str, group_id: Option<i64>, pinned: bool) -> Result<()> {
        if title.chars().count() > 200 {
            return Err("标题不能超过 200 个字符。".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let changed = self
            .conn
            .execute(
                "UPDATE clips SET title=?1,group_id=?2,pinned=?3 WHERE id=?4",
                params![title.trim(), group_id, pinned, id],
            )
            .map_err(sql_error)?;
        if changed == 0 {
            return Err("这条星笺已被删除。".into());
        }
        scan_clip(&tx, id, &self.privacy_rules()?, false)?;
        tx.commit().map_err(sql_error)?;
        Ok(())
    }

    pub fn mark_copied(&self, id: i64) -> Result<()> {
        self.conn
            .execute("UPDATE clips SET copy_count=copy_count+1 WHERE id=?1", [id])
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        if self.clip(id)?.keep_forever {
            return Err("这条星笺已设为永久保留，请先取消保留再删除。".into());
        }
        self.conn
            .execute("DELETE FROM clips WHERE id=?1 AND keep_forever=0", [id])
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM clips WHERE pinned=0 AND keep_forever=0", [])
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn set_keep_forever(&self, id: i64, keep_forever: bool) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE clips SET keep_forever=?1 WHERE id=?2",
                params![keep_forever, id],
            )
            .map_err(sql_error)?;
        if changed == 0 {
            return Err("这条星笺已被删除。".into());
        }
        Ok(())
    }

    fn database_bytes(&self) -> Result<u64> {
        let Some(path) = self.conn.path().filter(|path| !path.is_empty()) else {
            return Ok(0);
        };
        let database = fs::metadata(path)
            .map_err(|error| format!("无法读取数据库占用：{error}"))?
            .len();
        let wal = match fs::metadata(format!("{path}-wal")) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(format!("无法读取数据库日志占用：{error}")),
        };
        Ok(database.saturating_add(wal))
    }

    /// This only reads rows, file sizes and SQLite page statistics; it never prunes.
    pub fn preview_cleanup(&self, filter: &CleanupFilter) -> Result<CleanupSnapshot> {
        if !["all", "text", "link", "code", "color", "image", "file"]
            .contains(&filter.kind.as_str())
        {
            return Err("不支持的清理内容类型。".into());
        }
        let cutoff = now().saturating_sub(i64::from(filter.older_than_days) * 86_400_000);
        let mut total_count = 0;
        let mut protected_count = 0;
        let mut content_bytes = 0;
        let mut candidates = Vec::new();
        let mut items: Vec<CleanupItem> = Vec::new();
        let mut stmt = self.conn.prepare(CLIP_SELECT).map_err(sql_error)?;
        for row in stmt.query_map([], clip_from_row).map_err(sql_error)? {
            let clip = row.map_err(sql_error)?;
            total_count += 1;
            if clip.pinned || clip.keep_forever {
                protected_count += 1;
                continue;
            }
            let bytes = clip_bytes(&clip);
            if (filter.min_bytes == 0 || bytes > filter.min_bytes)
                && (filter.older_than_days == 0 || clip.updated_at < cutoff)
                && (filter.kind == "all" || clip.kind == filter.kind)
                && (filter.include_grouped || clip.group_id.is_none())
            {
                content_bytes += bytes;
                candidates.push(CleanupCandidate {
                    id: clip.id,
                    fingerprint: cleanup_fingerprint(&clip)?,
                });
                // Only the largest 100 display rows need titles. All candidates
                // retain just an ID and digest for the transactional recheck.
                let position =
                    items.partition_point(|item| (item.bytes, item.id) > (bytes, clip.id));
                if position < 100 {
                    items.insert(
                        position,
                        CleanupItem {
                            id: clip.id,
                            title: if clip.is_private {
                                "隐私记录".into()
                            } else {
                                clip.title
                            },
                            kind: clip.kind,
                            bytes,
                            updated_at: clip.updated_at,
                        },
                    );
                    items.truncate(100);
                }
            }
        }
        let page_size: u64 = self
            .conn
            .query_row("PRAGMA page_size", [], |row| row.get(0))
            .map_err(sql_error)?;
        let free_pages: u64 = self
            .conn
            .query_row("PRAGMA freelist_count", [], |row| row.get(0))
            .map_err(sql_error)?;
        let preview = CleanupPreview {
            token: format!(
                "{:x}-{:x}-{:x}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                CLEANUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ),
            total_count,
            protected_count,
            matched_count: candidates.len(),
            content_bytes,
            database_bytes: self.database_bytes()?,
            reusable_bytes: free_pages.saturating_mul(page_size),
            items,
            truncated: candidates.len() > 100,
        };
        Ok(CleanupSnapshot {
            preview,
            candidates,
        })
    }

    fn compact_database(&self) -> Result<()> {
        self.conn.execute_batch("VACUUM").map_err(sql_error)?;
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
            .map_err(sql_error)?;
        if busy != 0 {
            return Err("数据库日志仍被其他读取占用，尚未全部释放磁盘空间。".into());
        }
        Ok(())
    }

    pub fn execute_cleanup(
        &mut self,
        snapshot: CleanupSnapshot,
        compact: bool,
    ) -> Result<CleanupResult> {
        let before = self.database_bytes()?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        for expected in &snapshot.candidates {
            let current = tx
                .query_row(
                    &format!("{CLIP_SELECT} WHERE c.id=?1"),
                    [expected.id],
                    clip_from_row,
                )
                .optional()
                .map_err(sql_error)?;
            let current = current.ok_or(STALE_CLEANUP)?;
            if current.keep_forever
                || current.pinned
                || cleanup_fingerprint(&current)? != expected.fingerprint
            {
                return Err(STALE_CLEANUP.into());
            }
        }
        let mut deleted_count = 0;
        for clip in &snapshot.candidates {
            let deleted = tx
                .execute(
                    "DELETE FROM clips WHERE id=?1 AND pinned=0 AND keep_forever=0",
                    [clip.id],
                )
                .map_err(sql_error)?;
            if deleted != 1 {
                return Err(STALE_CLEANUP.into());
            }
            deleted_count += deleted;
        }
        tx.commit().map_err(sql_error)?;
        // After this point deletion succeeded. Compaction/stat failures are warnings,
        // never a false failure that invites the user to repeat destructive work.
        let mut warning = if compact {
            self.compact_database()
                .err()
                .map(|error| format!("记录已清理，但磁盘整理未完成：{error}"))
        } else {
            None
        };
        let after = match self.database_bytes() {
            Ok(bytes) => bytes,
            Err(error) => {
                let message = format!("{error}；最终占用暂沿用清理前数值，释放量尚无法测量。");
                warning = Some(match warning {
                    Some(warning) => format!("{warning}；{message}"),
                    None => message,
                });
                before
            }
        };
        Ok(CleanupResult {
            deleted_count,
            content_bytes: snapshot.preview.content_bytes,
            database_bytes_before: before,
            database_bytes_after: after,
            released_bytes: before.saturating_sub(after),
            warning,
        })
    }

    pub fn prune_history(&self) -> Result<usize> {
        Self::prune_with_settings(&self.conn, &self.settings()?, now())
    }

    fn prune_with_settings(
        conn: &Connection,
        settings: &Settings,
        timestamp: i64,
    ) -> Result<usize> {
        if settings.max_history == 0 && settings.retention_days == 0 {
            return Ok(0);
        }
        let cutoff = timestamp.saturating_sub(i64::from(settings.retention_days) * 86_400_000);
        // Protected and grouped snippets never consume the ordinary history quota.
        conn.execute("DELETE FROM clips WHERE pinned=0 AND keep_forever=0 AND group_id IS NULL AND (
            (?2 > 0 AND updated_at < ?3) OR (?1 > 0 AND id NOT IN
            (SELECT id FROM clips WHERE pinned=0 AND keep_forever=0 AND group_id IS NULL ORDER BY updated_at DESC,id DESC LIMIT ?1)))",
            params![settings.max_history as i64, settings.retention_days, cutoff]).map_err(sql_error)
    }

    pub fn create_group(&self, name: &str, color: &str) -> Result<Group> {
        let name = name.trim();
        validate_group(name, color)?;
        self.conn
            .execute(
                "INSERT INTO groups(name,color) VALUES(?1,?2)",
                params![name, color],
            )
            .map_err(|_| "分组创建失败，名称可能已存在。".to_string())?;
        Ok(Group {
            id: self.conn.last_insert_rowid(),
            name: name.into(),
            color: color.into(),
        })
    }

    pub fn delete_group(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM groups WHERE id=?1", [id])
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn export(&self) -> Result<String> {
        // Bound allocations before loading histories or image BLOBs. Serialized JSON is
        // checked again because base64 and metadata make it larger than the stored content.
        let (count, bytes): (usize, u64) = self.conn.query_row(
            "SELECT count(*),COALESCE(sum(CASE WHEN c.kind='image' THEN COALESCE(i.bytes,0) ELSE length(CAST(c.content AS BLOB)) END + length(CAST(c.title AS BLOB))),0) FROM clips c LEFT JOIN clip_images i ON i.clip_id=c.id",
            [], |row| Ok((row.get(0)?, row.get(1)?))).map_err(sql_error)?;
        let group_count: usize = self
            .conn
            .query_row("SELECT count(*) FROM groups", [], |row| row.get(0))
            .map_err(sql_error)?;
        check_backup_limits(
            usize::try_from(bytes).unwrap_or(usize::MAX),
            count,
            group_count,
        )?;
        let clips = self.clips()?;
        check_backup_image_limits(&clips)?;
        let groups = self.groups()?;
        let mut images = Vec::new();
        let mut encoded_bytes = 0usize;
        for clip in &clips {
            if clip.kind == "image" {
                let png = self.image_png(clip.id, false)?;
                let metadata = clip
                    .image
                    .as_ref()
                    .ok_or("图片元数据缺失，无法导出完整备份。")?;
                let (width, height, rgba) = images::decode_png(&png)?;
                if metadata.width != width
                    || metadata.height != height
                    || metadata.bytes != png.len() as u64
                    || images::fingerprint_rgba(width, height, &rgba)? != clip.content
                {
                    return Err("图片数据与元数据不符，未导出损坏的备份。".into());
                }
                let png = STANDARD.encode(png);
                encoded_bytes = encoded_bytes.saturating_add(png.len());
                check_backup_limits(encoded_bytes, 0, 0)?;
                images.push(BackupImage {
                    clip_id: clip.id,
                    png,
                });
            }
        }
        let json = serde_json::to_string_pretty(&Backup {
            format: "starclip".into(),
            version: 4,
            exported_at: now(),
            clips,
            groups,
            images,
            privacy_rules: Some(self.privacy_rules()?),
        })
        .map_err(|e| e.to_string())?;
        check_backup_limits(json.len(), 0, 0)?;
        Ok(json)
    }

    pub fn import(&mut self, json: &str) -> Result<usize> {
        check_backup_limits(json.len(), 0, 0)?;
        let backup: Backup = serde_json::from_str(json)
            .map_err(|_| "无法读取备份文件，请选择星笺导出的 JSON 文件。")?;
        if backup.format != "starclip"
            || ![1, 2, 3, 4].contains(&backup.version)
            || (backup.version == 1 && !backup.images.is_empty())
        {
            return Err("不支持的备份版本。".into());
        }
        check_backup_limits(json.len(), backup.clips.len(), backup.groups.len())?;
        if let Some(rules) = &backup.privacy_rules {
            rules.validate()?;
        }
        let current_rules = self.privacy_rules()?;
        let mut active_rules = current_rules.clone();
        if let Some(imported_rules) = &backup.privacy_rules {
            for (index, imported) in imported_rules.rules.iter().enumerate() {
                // Re-importing the same policy does not accumulate duplicate drafts.
                if active_rules.rules.iter().any(|local| {
                    local.name == imported.name
                        && local.condition == imported.condition
                        && local.value == imported.value
                        && local.tag == imported.tag
                        && local.mark_private == imported.mark_private
                }) {
                    continue;
                }
                let mut draft = imported.clone();
                draft.enabled = false;
                let mut suffix = index;
                loop {
                    draft.id = format!("import-{}-{suffix}", current_rules.revision);
                    if !active_rules.rules.iter().any(|rule| rule.id == draft.id) {
                        break;
                    }
                    suffix += 1;
                }
                active_rules.rules.push(draft);
            }
            if active_rules.rules != current_rules.rules {
                active_rules.revision = current_rules
                    .revision
                    .checked_add(1)
                    .ok_or("规则版本达到上限。")?;
            }
            active_rules.validate()?;
        }
        let mut group_ids = HashSet::new();
        for group in &backup.groups {
            validate_group(&group.name, &group.color)?;
            if !group_ids.insert(group.id) {
                return Err("备份存在重复分组编号。".into());
            }
        }
        let mut clip_ids = HashSet::new();
        for clip in &backup.clips {
            validate_content(&clip.content, &clip.title)?;
            validate_backup_tags(&clip.tags)?;
            validate_backup_tags(&clip.manual_tags)?;
            if clip.privacy_reasons.len() > 128
                || clip.privacy_reasons.iter().any(|reason| reason.len() > 512)
            {
                return Err("备份隐私原因无效。".into());
            }
            if !clip_ids.insert(clip.id) {
                return Err("备份存在重复星笺编号。".into());
            }
            if let Some(language) = &clip.language {
                validate_language(language)?;
            }
            if clip.group_id.is_some_and(|id| !group_ids.contains(&id))
                || clip.created_at < 0
                || clip.updated_at < 0
                || clip.copy_count < 0
            {
                return Err("备份包含无效分组或记录。".into());
            }
            if (clip.kind == "image"
                && (backup.version < 2 || clip.image.is_none() || clip.language.is_some()))
                || (clip.kind != "image" && clip.image.is_some())
            {
                return Err("备份图片类型或元数据无效。".into());
            }
            if clip.kind == "file" {
                // File records are validated without touching the disk: the
                // backup may come from a machine where the paths no longer exist.
                let entries = clip
                    .entries
                    .as_deref()
                    .ok_or("备份文件记录缺少路径列表。")?;
                if validate_backup_entries(entries)? != clip.content {
                    return Err("备份文件记录与路径列表不一致。".into());
                }
                if clip.language.is_some() {
                    return Err("备份文件记录不能同时是代码片段。".into());
                }
            } else if clip.entries.is_some() {
                return Err("备份中非文件记录带有路径列表。".into());
            }
        }
        let mut prepared = HashMap::new();
        let mut prepared_bytes = 0usize;
        check_backup_image_limits(&backup.clips)?;
        let image_clips: HashMap<_, _> = backup
            .clips
            .iter()
            .filter(|clip| clip.kind == "image")
            .map(|clip| (clip.id, clip))
            .collect();
        for image in backup.images {
            let clip = image_clips
                .get(&image.clip_id)
                .ok_or("备份图片没有对应的图片记录。")?;
            if prepared.contains_key(&image.clip_id) {
                return Err("备份存在重复图片。".into());
            }
            let png = STANDARD
                .decode(&image.png)
                .map_err(|_| "备份图片的 Base64 数据损坏。")?;
            let value = images::prepare_png(&png)?;
            let metadata = clip.image.as_ref().unwrap();
            if metadata.width != value.metadata().width
                || metadata.height != value.metadata().height
                || metadata.bytes != png.len() as u64
                || clip.content != value.digest()
            {
                return Err("备份图片的像素、尺寸或摘要不匹配。".into());
            }
            prepared_bytes = prepared_bytes.saturating_add(value.png.len());
            check_backup_limits(prepared_bytes, 0, 0)?;
            prepared.insert(image.clip_id, value);
        }
        if backup
            .clips
            .iter()
            .any(|clip| clip.kind == "image" && !prepared.contains_key(&clip.id))
        {
            return Err("备份缺少原始图片，未导入不完整记录。".into());
        }
        // All image bytes are validated before the transaction creates any group or clip.
        let tx = self.conn.transaction().map_err(sql_error)?;
        if active_rules != current_rules {
            tx.execute("INSERT INTO preferences(key,value) VALUES('privacy_rules_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&active_rules).map_err(|e|e.to_string())?]).map_err(sql_error)?;
        }
        let mut group_mapping = HashMap::new();
        for group in backup.groups {
            tx.execute(
                "INSERT INTO groups(name,color) VALUES(?1,?2) ON CONFLICT(name) DO NOTHING",
                params![group.name.trim(), group.color],
            )
            .map_err(sql_error)?;
            let id: i64 = tx
                .query_row(
                    "SELECT id FROM groups WHERE name=?1",
                    [group.name.trim()],
                    |row| row.get(0),
                )
                .map_err(sql_error)?;
            group_mapping.insert(group.id, id);
        }
        let mut imported = 0;
        for clip in backup.clips {
            let group_id = clip.group_id.and_then(|id| group_mapping.get(&id).copied());
            let is_image = clip.kind == "image";
            let is_file = clip.kind == "file";
            let predicate = if is_image {
                "kind='image'"
            } else if is_file {
                "kind='file'"
            } else {
                "kind NOT IN ('image','file')"
            };
            let existed: Option<i64> = tx
                .query_row(
                    &format!("SELECT id FROM clips WHERE content=?1 AND {predicate}"),
                    [&clip.content],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sql_error)?;
            let kind = if is_image {
                "image"
            } else if is_file {
                "file"
            } else if clip.language.is_some() {
                "code"
            } else {
                classify(&clip.content)
            };
            tx.execute(&format!("INSERT INTO clips(content,title,kind,pinned,group_id,created_at,updated_at,copy_count,language,keep_forever) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                ON CONFLICT(content) WHERE {predicate} DO UPDATE SET pinned=MAX(clips.pinned,excluded.pinned),
                keep_forever=MAX(clips.keep_forever,excluded.keep_forever),
                group_id=COALESCE(clips.group_id,excluded.group_id),
                title=CASE WHEN clips.title='' THEN excluded.title ELSE clips.title END,
                language=COALESCE(clips.language,excluded.language),
                kind=CASE WHEN clips.language IS NOT NULL OR excluded.language IS NOT NULL THEN 'code' ELSE clips.kind END"),
                params![clip.content, clip.title, kind, clip.pinned, group_id, clip.created_at, clip.updated_at, clip.copy_count, clip.language, clip.keep_forever]).map_err(sql_error)?;
            let id = existed.unwrap_or_else(|| tx.last_insert_rowid());
            if let Some(image) = prepared.remove(&clip.id) {
                Self::put_image(&tx, id, &image)?;
            }
            if let Some(entries) = clip.entries.as_deref() {
                // The merged row may already exist, so replace its list instead
                // of appending duplicates. Entry kinds come from the backup.
                tx.execute("DELETE FROM clip_files WHERE clip_id=?1", [id])
                    .map_err(sql_error)?;
                let mut statement = tx
                    .prepare("INSERT INTO clip_files(clip_id,position,path,entry_kind) VALUES(?1,?2,?3,?4)")
                    .map_err(sql_error)?;
                for (position, entry) in entries.iter().enumerate() {
                    statement
                        .execute(params![
                            id,
                            position as i64,
                            entry.path,
                            entry.entry_kind.as_str()
                        ])
                        .map_err(sql_error)?;
                }
            }
            let mut metadata = read_metadata(&tx, id)?;
            metadata.privacy_manual |=
                clip.privacy_manual || (clip.is_private && clip.privacy_reasons.is_empty());
            metadata.privacy_reasons =
                privacy::union(&metadata.privacy_reasons, &clip.privacy_reasons);
            let manual_tags = if backup.version >= 3 {
                &clip.manual_tags
            } else {
                &clip.tags
            };
            metadata.manual_tags = privacy::union(&metadata.manual_tags, manual_tags);
            // Imported scan version/classification cannot grant trust. Reclassify the final merged row.
            put_metadata(&tx, id, &metadata)?;
            scan_clip(&tx, id, &active_rules, false)?;
            if existed.is_none() {
                imported += 1;
            }
        }
        // Import keeps the entire backup; capture, startup and maintenance apply retention later.
        tx.commit().map_err(sql_error)?;
        Ok(imported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }

    #[test]
    fn window_startup_preferences_default_without_writing_and_accept_missing_fields() {
        let db = store();
        assert_eq!(
            db.window_startup_preferences().unwrap(),
            WindowStartupPreferences::default()
        );
        let count: i64 = db
            .conn
            .query_row(
                "SELECT count(*) FROM preferences WHERE key='window_startup_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
        for (json, expected) in [
            ("{}", WindowStartupPreferences::default()),
            (
                r#"{"startHidden":true}"#,
                WindowStartupPreferences {
                    start_hidden: true,
                    show_tray_icon: true,
                },
            ),
            (
                r#"{"showTrayIcon":false}"#,
                WindowStartupPreferences {
                    start_hidden: false,
                    show_tray_icon: false,
                },
            ),
        ] {
            db.conn.execute(
                "INSERT INTO preferences(key,value) VALUES('window_startup_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]
            ).unwrap();
            assert_eq!(db.window_startup_preferences().unwrap(), expected);
            let saved: String = db
                .conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='window_startup_v1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(saved, json, "reading defaults must not rewrite stored data");
        }
    }

    #[test]
    fn window_startup_preferences_persist_without_touching_capture_or_retention() {
        let scratch = Scratch::new();
        let path = scratch.0.join("window-startup.sqlite3");
        let db = Store::open(&path).unwrap();
        let saved = WindowStartupPreferences {
            start_hidden: true,
            show_tray_icon: false,
        };
        db.save_window_startup_preferences(&saved).unwrap();
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.window_startup_preferences().unwrap(), saved);
        let ordinary = r#"{"paused":false,"pauseRevision":9,"theme":"dark","layout":"grid","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [ordinary],
            )
            .unwrap();
        db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('启动偏好原创超期夹具','text',0,0)", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER forbid_startup_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'startup must not prune'); END;").unwrap();
        let before = db.settings().unwrap();
        db.save_window_startup_preferences(&WindowStartupPreferences::default())
            .unwrap();
        assert_eq!(db.settings().unwrap(), before);
        assert_eq!(db.clips().unwrap().len(), 1);
        let settings: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(settings, ordinary);
        let json: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='window_startup_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap(),
            serde_json::json!({ "startHidden": false, "showTrayIcon": true })
        );
        db.conn.execute_batch("CREATE TRIGGER fail_startup_preferences BEFORE UPDATE ON preferences WHEN NEW.key='window_startup_v1' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
        assert!(db.save_window_startup_preferences(&saved).is_err());
        assert_eq!(
            db.window_startup_preferences().unwrap(),
            WindowStartupPreferences::default()
        );
    }

    #[test]
    fn window_startup_preferences_report_corruption_without_resetting_it() {
        let db = store();
        for json in [
            "broken",
            "null",
            r#"{"startHidden":"true"}"#,
            r#"{"showTrayIcon":null}"#,
        ] {
            db.conn.execute(
                "INSERT INTO preferences(key,value) VALUES('window_startup_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]
            ).unwrap();
            assert!(db.window_startup_preferences().is_err());
            let saved: String = db
                .conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='window_startup_v1'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(saved, json);
        }
    }

    #[test]
    fn history_selected_details_are_bounded_fresh_and_omit_deleted_ids() {
        let store = store();
        let first = store.add("原创详情一", "旧标题", None).unwrap();
        let second = store.add("原创详情二", "待删除", None).unwrap();
        store.update(first.id, "新标题", None, true).unwrap();
        store.set_clip_privacy(first.id, true, false).unwrap();
        store
            .conn
            .execute("DELETE FROM clips WHERE id=?1", [second.id])
            .unwrap();
        let details = store
            .clip_details(&[second.id, first.id, first.id])
            .unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].title, "新标题");
        assert!(details[0].is_private);
        assert!(store.clip_details(&[]).unwrap().is_empty());
        assert!(store.clip_details(&[0]).is_err());
        assert!(store.clip_details(&vec![first.id; 501]).is_err());
    }

    #[test]
    fn history_page_bounds_content_and_searches_the_entire_store() {
        let store = store();
        let mut first_id = 0;
        for index in 0..123 {
            let clip = store
                .add(
                    &format!("分页原创内容 {index}"),
                    if index == 0 {
                        "ÉTOILE 100%_"
                    } else {
                        "原创固定样例"
                    },
                    None,
                )
                .unwrap();
            if index == 0 {
                first_id = clip.id;
            }
        }
        store
            .conn
            .execute("UPDATE clips SET updated_at=100", [])
            .unwrap();
        let first = store.history_page(&HistoryQuery::default()).unwrap();
        assert_eq!(first.clips.len(), 50);
        assert_eq!(first.history.total, 123);
        assert_eq!(first.history.counts["all"], 123);
        assert!(first.clips.windows(2).all(|pair| pair[0].id > pair[1].id));
        let last = store
            .history_page(&HistoryQuery {
                page: u32::MAX,
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(last.history.page, 3);
        assert_eq!(last.clips.len(), 23);
        assert!(!first
            .clips
            .iter()
            .any(|clip| last.clips.iter().any(|other| other.id == clip.id)));
        let search = store
            .history_page(&HistoryQuery {
                query: "étoile 100%_".into(),
                page: 3,
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(search.history.page, 1);
        assert_eq!(search.history.total, 1);
        assert_eq!(search.clips[0].id, first_id);
        let empty = store
            .history_page(&HistoryQuery {
                query: "并不存在的原创文字".into(),
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!((empty.history.page, empty.history.total), (1, 0));
        assert_eq!(
            store.clips().unwrap().len(),
            123,
            "paging never prunes retained history"
        );
        store
            .conn
            .execute(
                "DELETE FROM clips WHERE id IN (SELECT id FROM clips ORDER BY id LIMIT 23)",
                [],
            )
            .unwrap();
        assert_eq!(
            store
                .history_page(&HistoryQuery {
                    page: 3,
                    ..HistoryQuery::default()
                })
                .unwrap()
                .history
                .page,
            2,
            "deletion clamps an emptied last page"
        );
    }

    #[test]
    fn history_page_keeps_global_counts_facets_and_privacy_filters() {
        let store = store();
        let group = store.create_group("原创分页分组", "#778899").unwrap();
        let ordinary = store
            .add(
                "https://linux.do/original-page-fixture",
                "原创链接",
                Some(group.id),
            )
            .unwrap();
        store
            .set_clip_tags(ordinary.id, &["原创标签".into()])
            .unwrap();
        store
            .update(ordinary.id, "原创链接", Some(group.id), true)
            .unwrap();
        let private = store.add("测试普通内容", "人工保护样例", None).unwrap();
        store.set_clip_privacy(private.id, true, false).unwrap();
        let image = store
            .add_image(&image_fixture(44), "原创像素", None, false)
            .unwrap();
        let mut legacy = read_metadata(&store.conn, image.id).unwrap();
        legacy.privacy_reasons = vec!["图片尚未进行文字识别".into()];
        put_metadata(&store.conn, image.id, &legacy).unwrap();
        let query = HistoryQuery {
            filter: format!("group-{}", group.id),
            sources: vec!["host:linux.do".into()],
            tag: "原创标签".into(),
            privacy: "ordinary".into(),
            ..HistoryQuery::default()
        };
        let result = store.history_page(&query).unwrap();
        assert_eq!(result.clips.len(), 1);
        assert_eq!(result.clips[0].id, ordinary.id);
        assert_eq!(result.history.counts["all"], 3);
        assert_eq!(result.history.counts["pinned"], 1);
        assert_eq!(result.history.counts["link"], 1);
        assert_eq!(result.history.group_counts[&group.id], 1);
        assert!(result.history.tags.contains(&"原创标签".into()));
        assert!(result.history.hosts.contains(&"linux.do".into()));
        assert!(result.history.sources.contains(&"linuxdo".into()));
        let protected = store
            .history_page(&HistoryQuery {
                privacy: "private".into(),
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(
            protected
                .clips
                .iter()
                .map(|clip| clip.id)
                .collect::<Vec<_>>(),
            [private.id]
        );
        let ai = store
            .history_page(&HistoryQuery {
                ai_eligible: true,
                query: "原创标签".into(),
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(
            ai.clips.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            [ordinary.id]
        );
        let pending = store
            .history_page(&HistoryQuery {
                privacy: "pending".into(),
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(
            pending.clips.iter().map(|clip| clip.id).collect::<Vec<_>>(),
            [image.id]
        );
    }

    fn all_cleanup() -> CleanupFilter {
        CleanupFilter {
            min_bytes: 0,
            older_than_days: 0,
            kind: "all".into(),
            include_grouped: false,
        }
    }
    fn image_fixture(shade: u8) -> PreparedImage {
        images::prepare_rgba(2, 2, &[shade, 70, 180, 255].repeat(4)).unwrap()
    }
    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "starclip-cleanup-test-{}-{}",
                std::process::id(),
                CLEANUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cleanup_preview_masks_private_titles_without_changing_candidates() {
        let db = store();
        let clip = db
            .add("原创保护预览", "只在主动展开时显示的标题", None)
            .unwrap();
        db.set_clip_privacy(clip.id, true, false).unwrap();
        let preview = db.preview_cleanup(&all_cleanup()).unwrap().preview;
        assert_eq!(preview.matched_count, 1);
        assert_eq!(preview.items[0].id, clip.id);
        assert_eq!(preview.items[0].title, "隐私记录");
        assert_eq!(db.clip(clip.id).unwrap().title, "只在主动展开时显示的标题");
    }

    #[test]
    fn record_identity_survives_merge_but_never_reuses_a_deleted_grant() {
        let mut db = store();
        let original = db.add("记录身份原创夹具", "", None).unwrap();
        assert_eq!(original.record_identity.len(), 64);
        assert_eq!(
            db.add(&original.content, "", None).unwrap().record_identity,
            original.record_identity
        );
        let backup = db.export().unwrap();
        assert!(!backup.contains("recordIdentity"));
        db.import(&backup).unwrap();
        assert_eq!(
            db.clip(original.id).unwrap().record_identity,
            original.record_identity
        );
        db.delete(original.id).unwrap();
        let replacement = db.add("后来新增的另一条记录", "", None).unwrap();
        assert_eq!(
            replacement.id, original.id,
            "fixture must exercise SQLite rowid reuse"
        );
        assert_ne!(replacement.record_identity, original.record_identity);
        let mut imported = store();
        imported.import(&backup).unwrap();
        assert_ne!(
            imported.clips().unwrap()[0].record_identity,
            original.record_identity
        );
    }

    #[test]
    fn privacy_is_independent_of_labels_and_never_downgraded_by_merges() {
        let mut db = store();
        let clip = db.add("原创私密笔记", "", None).unwrap();
        assert!(!clip.is_private);
        assert_eq!(clip.scan_version, 1);
        db.set_clip_privacy(clip.id, true, false).unwrap();
        db.set_clip_tags(clip.id, &["资料".into()]).unwrap();
        let before = db.export().unwrap();
        assert!(db.add("原创私密笔记", "新标题", None).unwrap().is_private);
        db.set_clip_tags(clip.id, &[]).unwrap();
        assert!(db.clip(clip.id).unwrap().is_private);
        assert!(db.set_clip_privacy(clip.id, false, false).is_err());
        let mut untrusted: serde_json::Value = serde_json::from_str(&before).unwrap();
        untrusted["clips"][0]["privacyManual"] = false.into();
        untrusted["clips"][0]["isPrivate"] = false.into();
        untrusted["clips"][0]["privacyReasons"] = serde_json::json!([]);
        untrusted["clips"][0]["scanVersion"] = 999.into();
        db.import(&untrusted.to_string()).unwrap();
        assert!(db.clip(clip.id).unwrap().is_private);
        assert_eq!(db.clip(clip.id).unwrap().scan_version, 1);
        assert!(
            !db.set_clip_privacy(clip.id, false, true)
                .unwrap()
                .is_private
        );
        let sensitive = db.add("password = fixed-fixture-only", "", None).unwrap();
        assert!(sensitive.is_private);
        assert!(
            db.set_clip_privacy(sensitive.id, false, true)
                .unwrap()
                .is_private
        );
        db.save_code(
            Some(sensitive.id),
            "",
            "let safe_fixture = 42;",
            "javascript",
            None,
        )
        .unwrap();
        assert!(
            db.clip(sensitive.id).unwrap().is_private,
            "editing cannot silently remove prior protection"
        );
        assert!(
            !db.set_clip_privacy(sensitive.id, false, true)
                .unwrap()
                .is_private
        );
    }

    #[test]
    fn legacy_image_pending_marker_is_not_privacy_but_actual_protection_survives() {
        let db = store();
        let image = db
            .add_image(&image_fixture(21), "原创截图", None, false)
            .unwrap();
        assert!(!image.is_private);
        assert_eq!(image.scan_version, 0);
        let mut metadata = read_metadata(&db.conn, image.id).unwrap();
        metadata.privacy_reasons = vec!["图片尚未进行文字识别".into()];
        put_metadata(&db.conn, image.id, &metadata).unwrap();
        let old_json: String = db
            .conn
            .query_row(
                "SELECT metadata FROM clip_privacy WHERE clip_id=?1",
                [image.id],
                |row| row.get(0),
            )
            .unwrap();
        let visible = db.clip(image.id).unwrap();
        assert!(!visible.is_private);
        assert!(visible.privacy_reasons.is_empty());
        assert_eq!(
            old_json,
            db.conn
                .query_row::<String, _, _>(
                    "SELECT metadata FROM clip_privacy WHERE clip_id=?1",
                    [image.id],
                    |row| row.get(0)
                )
                .unwrap(),
            "reading does not rewrite user data"
        );
        let mut imported = store();
        let mut old_backup: serde_json::Value =
            serde_json::from_str(&db.export().unwrap()).unwrap();
        old_backup["clips"][0]["isPrivate"] = true.into();
        old_backup["clips"][0]["privacyReasons"] = serde_json::json!(["图片尚未进行文字识别"]);
        imported.import(&old_backup.to_string()).unwrap();
        assert!(!imported.clips().unwrap()[0].is_private);
        old_backup["clips"][0]["privacyManual"] = true.into();
        imported.import(&old_backup.to_string()).unwrap();
        assert!(imported.clips().unwrap()[0].privacy_manual);
        metadata.privacy_manual = true;
        put_metadata(&db.conn, image.id, &metadata).unwrap();
        assert!(db.clip(image.id).unwrap().is_private);
        metadata.privacy_manual = false;
        metadata.privacy_reasons.push("自定义规则：原创规则".into());
        put_metadata(&db.conn, image.id, &metadata).unwrap();
        assert_eq!(
            db.clip(image.id).unwrap().privacy_reasons,
            vec!["自定义规则：原创规则"]
        );
        assert!(db.clip(image.id).unwrap().is_private);
        db.update(image.id, "api_key=original-fixture-only", None, false)
            .unwrap();
        assert!(db
            .clip(image.id)
            .unwrap()
            .privacy_reasons
            .contains(&"凭据字段".into()));
    }

    #[test]
    fn privacy_rules_rescan_preview_is_readonly_and_stale_application_is_rejected() {
        let db = store();
        let clip = db
            .add("原创内部计划 https://linux.do/t/fixture", "", None)
            .unwrap();
        let mut rules = db.privacy_rules().unwrap();
        rules.rules.push(privacy::PrivacyRule {
            id: "work".into(),
            name: "内部".into(),
            enabled: true,
            condition: "keyword".into(),
            value: "内部".into(),
            tag: Some("工作".into()),
            mark_private: true,
        });
        rules = db.save_privacy_rules(&rules).unwrap();
        assert_eq!(rules.revision, 2);
        assert_eq!(db.clip(clip.id).unwrap().scan_version, 1);
        let preview = db.privacy_rescan_preview().unwrap();
        assert_eq!(preview.newly_private_count, 1);
        assert!(
            !db.clip(clip.id).unwrap().is_private,
            "preview must not persist a scan"
        );
        db.add("预览后的新记录", "", None).unwrap();
        assert!(db.privacy_rescan_apply(&preview.token).is_err());
        let preview = db.privacy_rescan_preview().unwrap();
        db.privacy_rescan_apply(&preview.token).unwrap();
        let scanned = db.clip(clip.id).unwrap();
        assert!(scanned.is_private);
        assert_eq!(scanned.scan_version, 2);
        assert_eq!(scanned.tags, vec!["工作"]);
        assert!(scanned.manual_tags.is_empty());
        db.set_clip_tags(clip.id, &["手动".into()]).unwrap();
        assert_eq!(db.clip(clip.id).unwrap().tags, vec!["工作", "手动"]);
        rules.rules[0].enabled = false;
        let current = db.save_privacy_rules(&rules).unwrap();
        assert!(db.save_privacy_rules(&rules).is_err());
        assert_eq!(current.revision, 3);
        db.privacy_rescan_apply(&db.privacy_rescan_preview().unwrap().token)
            .unwrap();
        assert!(
            db.clip(clip.id).unwrap().is_private,
            "rule removal does not remove protection"
        );
        assert_eq!(db.clip(clip.id).unwrap().tags, vec!["手动"]);
        assert!(
            !db.set_clip_privacy(clip.id, false, true)
                .unwrap()
                .is_private
        );
    }

    #[test]
    fn privacy_backup_v3_roundtrip_reclassifies_and_keeps_imported_rules_disabled() {
        let mut source = store();
        let mut rules = source.privacy_rules().unwrap();
        rules.rules.push(privacy::PrivacyRule {
            id: "work".into(),
            name: "内部".into(),
            enabled: true,
            condition: "keyword".into(),
            value: "内部".into(),
            tag: Some("工作".into()),
            mark_private: true,
        });
        source.save_privacy_rules(&rules).unwrap();
        let original = source
            .add(
                "原创内部记录 https://www.bilibili.com/video/fixture",
                "",
                None,
            )
            .unwrap();
        let manual = source.add("原创手动保护记录", "", None).unwrap();
        source.set_clip_privacy(manual.id, true, false).unwrap();
        source.set_clip_tags(manual.id, &["手动".into()]).unwrap();
        let json = source.export().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&json).unwrap()["version"],
            4
        );
        let mut destination = store();
        destination.import(&json).unwrap();
        let imported = destination
            .clips()
            .unwrap()
            .into_iter()
            .find(|clip| clip.content == original.content)
            .unwrap();
        assert!(imported.is_private);
        assert!(
            imported.tags.is_empty(),
            "automatic tags are recomputed under local active rules"
        );
        assert_eq!(imported.link_sources, vec!["bilibili"]);
        assert_eq!(
            imported.scan_version,
            destination.privacy_rules().unwrap().revision,
            "imported scan version must not be trusted"
        );
        let local_rules = destination.privacy_rules().unwrap();
        assert_eq!(local_rules.rules.len(), 1);
        assert!(!local_rules.rules[0].enabled);
        assert!(local_rules.rules[0].id.starts_with("import-"));
        destination.import(&json).unwrap();
        assert_eq!(
            destination.privacy_rules().unwrap().rules.len(),
            1,
            "reimport must not duplicate drafts"
        );
        let imported_manual = destination
            .clips()
            .unwrap()
            .into_iter()
            .find(|clip| clip.content == manual.content)
            .unwrap();
        assert!(imported_manual.privacy_manual);
        assert_eq!(imported_manual.manual_tags, vec!["手动"]);
        // Real v1 input omits all privacy metadata; it still goes through the detector.
        let legacy = serde_json::json!({"format":"starclip","version":1,"exportedAt":0,"groups":[],"clips":[{"id":99,"content":"https://fixture.test/?api_key=fixture","title":"","kind":"link","pinned":false,"groupId":null,"createdAt":0,"updatedAt":0,"copyCount":0}]});
        source.import(&legacy.to_string()).unwrap();
        assert!(
            source
                .clips()
                .unwrap()
                .iter()
                .find(|clip| clip.content.starts_with("https://fixture"))
                .unwrap()
                .is_private
        );
    }

    #[test]
    fn privacy_migration_scans_v1_and_full_snapshot_preserves_metadata() {
        let scratch = Scratch::new();
        let path = scratch.0.join("privacy-upgrade.sqlite3");
        let db = Store::open(&path).unwrap();
        let clip = db.add("password=original-fixed-fixture", "", None).unwrap();
        db.set_clip_tags(clip.id, &["备份测试".into()]).unwrap();
        let snapshot = scratch.0.join("snapshot.sqlite3");
        db.snapshot(&snapshot).unwrap();
        let restored = Store::open(&snapshot).unwrap();
        assert!(restored.clip(clip.id).unwrap().is_private);
        assert_eq!(
            restored.clip(clip.id).unwrap().manual_tags,
            vec!["备份测试"]
        );
        drop(restored);
        drop(db);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE clip_privacy; PRAGMA user_version=1;")
            .unwrap();
        drop(conn);
        let upgraded = Store::open(&path).unwrap();
        assert_eq!(upgraded.schema_version().unwrap(), 3);
        assert!(upgraded.clip(clip.id).unwrap().is_private);
        assert_eq!(upgraded.clip(clip.id).unwrap().scan_version, 1);
        assert_eq!(fs::read_dir(scratch.0.join("recovery")).unwrap().count(), 1);
    }

    #[test]
    fn privacy_failure_rolls_back_capture_import_code_and_schema_upgrade() {
        let mut db = store();
        let code = db
            .save_code(None, "安全代码", "const fixture = 7;", "javascript", None)
            .unwrap();
        let old = db.export().unwrap();
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('privacy_rules_v1','broken')",
                [],
            )
            .unwrap();
        assert!(db.add("不能半写入的记录", "", None).is_err());
        assert!(db
            .save_code(
                Some(code.id),
                "新标题",
                "const fixture = 8;",
                "javascript",
                None
            )
            .is_err());
        assert!(db.import(&old).is_err());
        assert_eq!(db.clips().unwrap().len(), 1);
        assert_eq!(db.clip(code.id).unwrap().content, "const fixture = 7;");
        let scratch = Scratch::new();
        let path = scratch.0.join("rollback.sqlite3");
        let store = Store::open(&path).unwrap();
        store.add("升级回滚原创夹具", "", None).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("DROP TABLE clip_privacy; PRAGMA user_version=1; INSERT INTO preferences(key,value) VALUES('privacy_rules_v1','broken');").unwrap();
        drop(conn);
        let before = fs::read(&path).unwrap();
        assert!(Store::open(&path).err().unwrap().contains("恢复点已保留"));
        assert_eq!(fs::read(&path).unwrap(), before);
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        assert_eq!(Store::check_schema_version(&conn).unwrap(), 1);
        assert!(conn.prepare("SELECT * FROM clip_privacy").is_err());
    }

    #[test]
    fn appearance_is_independent_atomic_persistent_and_in_full_snapshots() {
        let scratch = Scratch::new();
        let path = scratch.0.join("appearance.sqlite3");
        let db = Store::open(&path).unwrap();
        assert_eq!(db.appearance().unwrap(), Appearance::default());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT count(*) FROM preferences WHERE key='appearance_v1'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let png = images::prepare_rgba(2, 1, &[80, 120, 200, 0, 160, 80, 220, 255]).unwrap();
        let base64 = STANDARD.encode(&png.png);
        db.save_appearance(
            &serde_json::json!({"accent":"#AABBCC", "background":"gradient", "angle":45}),
        )
        .unwrap();
        let saved = db
            .import_appearance_sticker(&base64, "原创小星星.png")
            .unwrap();
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.appearance().unwrap(), saved);
        let snapshot = scratch.0.join("appearance-snapshot.sqlite3");
        db.snapshot(&snapshot).unwrap();
        assert_eq!(Store::open(&snapshot).unwrap().appearance().unwrap(), saved);
        let ordinary = r#"{"paused":false,"theme":"dark","layout":"horizontal","maxHistory":1,"shortcut":"CommandOrControl+Shift+V","retentionDays":1,"futurePreference":"preserve"}"#;
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [ordinary],
            )
            .unwrap();
        db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('外观测试中的原创过期记录','text',0,0)", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER forbid_appearance_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'appearance must not prune'); END;").unwrap();
        let disabled = db
            .save_appearance(&serde_json::json!({"stickerEnabled":false, "colorEnd":"#FFFCDC"}))
            .unwrap();
        assert!(!disabled.sticker_enabled);
        assert_eq!(disabled.sticker_data_url, saved.sticker_data_url);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='settings'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            ordinary
        );
        assert_eq!(db.clips().unwrap().len(), 1);
        assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
        assert!(!db.export().unwrap().contains("appearance_v1"));
        assert!(db
            .import_appearance_sticker("invalid", "新贴纸.png")
            .is_err());
        assert!(db
            .save_appearance(&serde_json::json!({"angle":360}))
            .is_err());
        assert_eq!(db.appearance().unwrap(), disabled);
        db.conn.execute_batch("CREATE TRIGGER fail_appearance_write BEFORE UPDATE ON preferences WHEN NEW.key='appearance_v1' BEGIN SELECT RAISE(ABORT, 'fixture disk error'); END;").unwrap();
        assert!(db.import_appearance_sticker(&base64, "替换图.png").is_err());
        assert!(db.remove_appearance_sticker().is_err());
        assert_eq!(db.appearance().unwrap(), disabled);
        db.conn
            .execute_batch("DROP TRIGGER fail_appearance_write;")
            .unwrap();
        let removed = db.remove_appearance_sticker().unwrap();
        assert_eq!(removed.accent, disabled.accent);
        assert!(!removed.sticker_enabled);
        assert!(removed.sticker_data_url.is_none());
        assert_eq!(db.clips().unwrap().len(), 1);
        db.conn
            .execute(
                "UPDATE preferences SET value='{}' WHERE key='appearance_v1'",
                [],
            )
            .unwrap();
        assert!(db.appearance().is_err());
        assert!(db
            .save_appearance(&serde_json::json!({"angle":90}))
            .is_err());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='appearance_v1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "{}"
        );
    }

    #[test]
    fn starpet_growth_is_atomic_independent_and_revisioned() {
        use crate::starpet_growth::{encode_save, GrowthChange, GrowthState};
        let scratch = Scratch::new();
        let path = scratch.0.join("growth.sqlite3");
        let db = Store::open(&path).unwrap();
        let preferences = StarpetPreferences {
            enabled: true,
            mood: "reading".into(),
            color: "#AADDEE".into(),
            size: 140,
        };
        db.save_starpet_preferences(&preferences).unwrap();
        let settings = db.settings().unwrap();
        assert!(settings.paused);
        assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT count(*) FROM preferences WHERE key='starpet_growth_v1'",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            0
        );
        db.conn.execute_batch("CREATE TRIGGER growth_must_not_touch_clips BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'must not prune'); END;").unwrap();
        let today = "2026-10-26";
        let enabled = db
            .change_starpet_growth_on(GrowthChange::Enabled(true), today)
            .unwrap();
        assert_eq!(enabled.revision, 1);
        let first = db
            .change_starpet_growth_on(GrowthChange::Interaction("greet"), today)
            .unwrap();
        assert_eq!(first.revision, 2);
        for _ in 0..5 {
            assert_eq!(
                db.change_starpet_growth_on(GrowthChange::Interaction("greet"), today)
                    .unwrap(),
                first
            );
        }
        let second = db
            .change_starpet_growth_on(GrowthChange::Interaction("highfive"), today)
            .unwrap();
        assert_eq!((second.xp, second.revision), (25, 3));
        assert_eq!(db.starpet_preferences().unwrap(), preferences);
        assert_eq!(db.settings().unwrap(), settings);
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.starpet_growth().unwrap(), second);
        assert_eq!(db.starpet_preferences().unwrap(), preferences);
        let saved = encode_save(second.clone()).unwrap();
        let grown = db
            .change_starpet_growth_on(GrowthChange::Interaction("stretch"), today)
            .unwrap();
        assert!(db
            .change_starpet_growth_on(
                GrowthChange::Import {
                    data: &saved,
                    expected_revision: second.revision
                },
                today
            )
            .is_err());
        assert_eq!(db.starpet_growth().unwrap(), grown);
        let restored = db
            .change_starpet_growth_on(
                GrowthChange::Import {
                    data: &saved,
                    expected_revision: grown.revision,
                },
                today,
            )
            .unwrap();
        assert_eq!((restored.xp, restored.revision), (25, grown.revision + 1));
        db.conn.execute_batch("CREATE TRIGGER fail_growth_write BEFORE UPDATE ON preferences WHEN NEW.key='starpet_growth_v1' BEGIN SELECT RAISE(ABORT, 'fixture disk error'); END;").unwrap();
        assert!(db
            .change_starpet_growth_on(GrowthChange::Interaction("stretch"), today)
            .is_err());
        assert_eq!(db.starpet_growth().unwrap(), restored);
        db.conn
            .execute_batch("DROP TRIGGER fail_growth_write;")
            .unwrap();
        let reset = db
            .change_starpet_growth_on(
                GrowthChange::Reset {
                    expected_revision: restored.revision,
                },
                today,
            )
            .unwrap();
        assert_eq!(
            reset,
            GrowthState {
                revision: restored.revision + 1,
                ..GrowthState::default()
            }
        );
        assert_eq!(db.settings().unwrap(), settings);
        assert_eq!(db.starpet_preferences().unwrap(), preferences);
    }

    #[test]
    fn starpet_preferences_are_optional_persistent_and_independent_of_capture_and_retention() {
        let scratch = Scratch::new();
        let path = scratch.0.join("starpet.sqlite3");
        let fresh = Store::open(&path).unwrap();
        assert_eq!(
            fresh.starpet_preferences().unwrap(),
            StarpetPreferences::default()
        );
        assert_eq!(
            fresh
                .conn
                .query_row(
                    "SELECT count(*) FROM preferences WHERE key='starpet_v1'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        fresh
            .conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('starpet_v1','{}')",
                [],
            )
            .unwrap();
        assert_eq!(
            fresh.starpet_preferences().unwrap(),
            StarpetPreferences::default()
        );
        assert_eq!(
            fresh
                .conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='starpet_v1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "{}",
            "reading defaults must not rewrite existing data"
        );
        let legacy = r##"{"enabled":true,"mood":"sleepy"}"##;
        fresh
            .conn
            .execute(
                "UPDATE preferences SET value=?1 WHERE key='starpet_v1'",
                [legacy],
            )
            .unwrap();
        assert_eq!(
            fresh.starpet_preferences().unwrap(),
            StarpetPreferences {
                enabled: true,
                mood: "sleepy".into(),
                color: "#FFD66E".into(),
                size: 100,
            }
        );
        assert_eq!(
            fresh
                .conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='starpet_v1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            legacy
        );
        let preferences = StarpetPreferences {
            enabled: true,
            mood: "curious".into(),
            color: "#8CB9BD".into(),
            size: 120,
        };
        fresh.save_starpet_preferences(&preferences).unwrap();
        drop(fresh);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.starpet_preferences().unwrap(), preferences);

        let settings = Settings {
            paused: false,
            retention_days: 1,
            max_history: 50,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [&json],
            )
            .unwrap();
        db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('星伴表情变化不能删除这条原创超期片段','text',0,0)", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER forbid_starpet_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'starpet must not prune'); END;").unwrap();
        assert_eq!(STARPET_MOODS.len(), 32);
        for mood in STARPET_MOODS {
            let value = StarpetPreferences {
                enabled: *mood != "sleeping",
                mood: (*mood).into(),
                color: preferences.color.clone(),
                size: preferences.size,
            };
            db.save_starpet_preferences(&value).unwrap();
            assert_eq!(db.starpet_preferences().unwrap(), value);
        }
        assert_eq!(db.settings().unwrap(), settings);
        assert_eq!(db.clips().unwrap().len(), 1);
        assert!(!db.onboarding_completed().unwrap());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='settings'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            json
        );
        let old = db.starpet_preferences().unwrap();
        for mood in ["", "unknown", "IDLE", "idle "] {
            assert!(db
                .save_starpet_preferences(&StarpetPreferences {
                    enabled: true,
                    mood: mood.into(),
                    color: old.color.clone(),
                    size: old.size,
                })
                .is_err());
            assert_eq!(db.starpet_preferences().unwrap(), old);
        }
        for (patch, expected) in [
            (
                r#"{"enabled":false}"#,
                StarpetPreferences {
                    enabled: false,
                    ..old.clone()
                },
            ),
            (
                r#"{"mood":"greet"}"#,
                StarpetPreferences {
                    enabled: false,
                    mood: "greet".into(),
                    color: old.color.clone(),
                    size: old.size,
                },
            ),
            (
                r##"{"color":"#aabbcc"}"##,
                StarpetPreferences {
                    enabled: false,
                    mood: "greet".into(),
                    color: "#AABBCC".into(),
                    size: old.size,
                },
            ),
            (
                r#"{"size":160}"#,
                StarpetPreferences {
                    enabled: false,
                    mood: "greet".into(),
                    color: "#AABBCC".into(),
                    size: 160,
                },
            ),
        ] {
            let patch: StarpetPreferencePatch = serde_json::from_str(patch).unwrap();
            let mut value = db.starpet_preferences().unwrap();
            patch.apply_to(&mut value);
            db.save_starpet_preferences(&value).unwrap();
            assert_eq!(db.starpet_preferences().unwrap(), expected);
        }
        let old = db.starpet_preferences().unwrap();
        for size in [0, 69, 161, u16::MAX] {
            assert!(db
                .save_starpet_preferences(&StarpetPreferences {
                    size,
                    ..old.clone()
                })
                .is_err());
            assert_eq!(db.starpet_preferences().unwrap(), old);
        }
        for json in [r#"{"size":-1}"#, r#"{"size":100.5}"#, r#"{"size":65536}"#] {
            assert!(serde_json::from_str::<StarpetPreferencePatch>(json).is_err());
        }
        assert_eq!(db.settings().unwrap(), settings);
        assert_eq!(
            db.clips().unwrap().len(),
            1,
            "size changes must not run retention"
        );
        for color in [
            "",
            "#fff",
            "112233",
            "#1234567",
            "#12345g",
            "#１２３",
            " #123456",
            "#123456\n",
        ] {
            assert!(db
                .save_starpet_preferences(&StarpetPreferences {
                    color: color.into(),
                    ..old.clone()
                })
                .is_err());
            assert_eq!(db.starpet_preferences().unwrap(), old);
        }
        db.conn.execute_batch("CREATE TRIGGER fail_starpet_save BEFORE UPDATE ON preferences WHEN NEW.key='starpet_v1' BEGIN SELECT RAISE(ABORT, 'fixture disk error'); END;").unwrap();
        assert!(db.save_starpet_preferences(&preferences).is_err());
        assert_eq!(
            db.starpet_preferences().unwrap(),
            old,
            "failed persistence retains the complete previous preference"
        );
    }

    #[test]
    fn starpet_geometry_anchors_bottom_center_and_stays_in_work_area() {
        for (size, expected) in [
            (70, (176.0, 206.0)),
            (100, (230.0, 260.0)),
            (160, (338.0, 368.0)),
        ] {
            let preferences = StarpetPreferences {
                size,
                ..StarpetPreferences::default()
            };
            assert!(preferences.validate().is_ok());
            assert_eq!(preferences.window_size(), expected);
        }
        let area = (0.0, 0.0, 1920.0, 1080.0);
        let enlarged =
            starpet_resize_position((500.0, 500.0), (230.0, 260.0), (338.0, 368.0), area);
        assert_eq!(enlarged, (446.0, 392.0));
        assert_eq!(
            starpet_resize_position(enlarged, (338.0, 368.0), (230.0, 260.0), area),
            (500.0, 500.0)
        );
        assert_eq!(
            starpet_resize_position((1690.0, 820.0), (230.0, 260.0), (338.0, 368.0), area),
            (1582.0, 712.0)
        );
        assert_eq!(
            starpet_resize_position(
                (-1900.0, -180.0),
                (230.0, 260.0),
                (338.0, 368.0),
                (-1920.0, -200.0, 1920.0, 1080.0)
            ),
            (-1920.0, -200.0)
        );
        assert_eq!(
            starpet_resize_position(
                (1000.0, 1000.0),
                (460.0, 520.0),
                (676.0, 736.0),
                (0.0, 0.0, 3840.0, 2160.0)
            ),
            (892.0, 784.0),
            "geometry uses physical pixels consistently on HiDPI displays"
        );
        assert_eq!(
            starpet_resize_position(
                (20.0, 30.0),
                (10.0, 10.0),
                (200.0, 200.0),
                (10.0, 20.0, 100.0, 100.0)
            ),
            (10.0, 20.0)
        );
    }

    #[test]
    fn telemetry_preferences_persist_without_retention_or_other_preferences() {
        let scratch = Scratch::new();
        let path = scratch.0.join("telemetry-fixture.sqlite3");
        let db = Store::open(&path).unwrap();
        assert_eq!(
            db.telemetry_preferences().unwrap(),
            TelemetryPreferences::default()
        );
        let session = db.set_telemetry_mode(TelemetryMode::Session).unwrap();
        assert_eq!(session.mode, TelemetryMode::Session);
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.telemetry_preferences().unwrap(), session);
        let ordinary = r#"{"paused":false,"theme":"dark","layout":"grid","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [ordinary],
            )
            .unwrap();
        db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('统计偏好原创超期夹具','text',0,0)", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER forbid_telemetry_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'telemetry must not prune'); END;").unwrap();
        let disabled = db.set_telemetry_mode(TelemetryMode::Off).unwrap();
        assert_eq!(disabled.mode, TelemetryMode::Off);
        assert!(disabled.revision > session.revision);
        let saved: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, ordinary);
        assert_eq!(db.clips().unwrap().len(), 1);
        assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
        assert_eq!(
            db.update_preferences().unwrap(),
            UpdatePreferences::default()
        );
        let saved: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='telemetry_preferences_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            json.as_object().unwrap().len(),
            2,
            "no session ID or event queue persisted"
        );
        assert_eq!(
            json["mode"], "off",
            "the old boolean switch is never written"
        );
        db.conn.execute_batch("CREATE TRIGGER fail_telemetry_preferences BEFORE UPDATE ON preferences WHEN NEW.key='telemetry_preferences_v1' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
        assert!(db.set_telemetry_mode(TelemetryMode::Installation).is_err());
        assert_eq!(db.telemetry_preferences().unwrap(), disabled);
    }

    #[test]
    fn telemetry_default_only_applies_to_missing_preferences_not_opt_out_or_corruption() {
        let scratch = Scratch::new();
        let path = scratch.0.join("telemetry-default-fixture.sqlite3");
        let db = Store::open(&path).unwrap();
        assert_eq!(
            db.telemetry_preferences().unwrap().mode,
            TelemetryMode::Session,
            "a new installation starts per session, never per installation"
        );
        let opted_out = db.set_telemetry_mode(TelemetryMode::Off).unwrap();
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.telemetry_preferences().unwrap(), opted_out);
        assert_eq!(db.telemetry_preferences().unwrap().mode, TelemetryMode::Off);
        for json in [
            "broken",
            r#"{"mode":"off"}"#,
            r#"{"revision":0,"mode":"installation","enabled":true,"extra":1}"#,
            r#"{"revision":0,"mode":"forever"}"#,
            r#"{"revision":18446744073709551615,"mode":"off"}"#,
        ] {
            db.conn
                .execute(
                    "UPDATE preferences SET value=?1 WHERE key='telemetry_preferences_v1'",
                    [json],
                )
                .unwrap();
            assert!(
                db.telemetry_preferences().is_err(),
                "corruption must not act like a missing preference"
            );
        }
        assert_eq!(
            db.set_telemetry_mode(TelemetryMode::Off).unwrap().mode,
            TelemetryMode::Off,
            "explicit local choice can repair damaged preferences"
        );
        db.conn.execute_batch("DROP TABLE preferences").unwrap();
        assert!(
            db.telemetry_preferences().is_err(),
            "database read failures must not use the default"
        );
    }

    #[test]
    fn update_preferences_are_independent_persistent_and_daily_attempts_survive_failures() {
        let scratch = Scratch::new();
        let path = scratch.0.join("update-fixture.sqlite3");
        let db = Store::open(&path).unwrap();
        assert_eq!(
            db.update_preferences().unwrap(),
            UpdatePreferences::default()
        );
        let now = 1_000_000;
        let (attempt, started) = db.record_update_check(now, true).unwrap();
        assert!(started);
        assert_eq!(attempt.last_check_at, Some(now));
        assert!(attempt.revision > 0);
        // No completion is recorded: failure and app exit still throttle retries.
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(
            db.record_update_check(now + 1, true).unwrap(),
            (attempt.clone(), false)
        );
        assert!(
            db.record_update_check(now + AUTO_UPDATE_INTERVAL_MS, true)
                .unwrap()
                .1
        );
        let last = db.update_preferences().unwrap();
        let (rebased, started) = db.record_update_check(now, true).unwrap();
        assert!(!started);
        assert!(rebased.revision > last.revision);
        assert_eq!(rebased.last_check_at, Some(now));
        assert!(!db.record_update_check(now + 1, true).unwrap().1);

        let disabled = db.set_auto_update_check(false).unwrap();
        assert!(!disabled.auto_check);
        assert_eq!(
            db.record_update_check(now + 2 * AUTO_UPDATE_INTERVAL_MS, true)
                .unwrap(),
            (disabled, false)
        );
        assert!(
            db.record_update_check(now + 2, false).unwrap().1,
            "manual checks ignore auto setting and interval"
        );
        let skipped = db.skip_update_version("0.3.0").unwrap();
        assert_eq!(skipped.skipped_version.as_deref(), Some("0.3.0"));
        assert!(!skipped.auto_check);
        drop(db);
        let db = Store::open(&path).unwrap();
        assert_eq!(db.update_preferences().unwrap(), skipped);

        let ordinary = r#"{"paused":false,"theme":"dark","layout":"grid","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [ordinary],
            )
            .unwrap();
        db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('更新偏好原创超期夹具','text',0,0)", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER forbid_update_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'updates must not prune'); END;").unwrap();
        db.set_auto_update_check(true).unwrap();
        db.skip_update_version("0.4.0").unwrap();
        db.record_update_check(now + 3, false).unwrap();
        let saved: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, ordinary);
        assert_eq!(db.clips().unwrap().len(), 1);
        assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
        let before = db.update_preferences().unwrap();
        db.conn.execute_batch("CREATE TRIGGER fail_update_preferences BEFORE UPDATE ON preferences WHEN NEW.key='update_preferences_v1' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
        assert!(db.set_auto_update_check(false).is_err());
        assert!(db.record_update_check(now + 4, false).is_err());
        assert_eq!(db.update_preferences().unwrap(), before);
    }

    #[test]
    fn immediate_theme_change_preserves_legacy_preferences_and_never_prunes() {
        let scratch = Scratch::new();
        let path = scratch.0.join("theme.sqlite3");
        let fresh = Store::open(&path).unwrap();
        assert_eq!(fresh.settings().unwrap(), Settings::default());
        let initial = fresh.set_theme("white").unwrap();
        assert_eq!(
            initial,
            Settings {
                theme: "white".into(),
                ..Settings::default()
            }
        );
        assert!(!fresh.onboarding_completed().unwrap());
        drop(fresh);
        assert_eq!(Store::open(&path).unwrap().settings().unwrap(), initial);
        let db = store();
        let legacy = r#"{"paused":false,"theme":"dark","layout":"horizontal","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1,"futurePreference":"preserve"}"#;
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [legacy],
            )
            .unwrap();
        db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('只换主题不能清理这条原创超期记录','text',0,0)", []).unwrap();
        db.conn.execute_batch("CREATE TRIGGER forbid_theme_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'theme must not prune'); END;").unwrap();
        let mut expected: serde_json::Value = serde_json::from_str(legacy).unwrap();
        for theme in ["light", "white", "forest", "rose", "system", "dark"] {
            let settings = db.set_theme(theme).unwrap();
            assert_eq!(settings.theme, theme);
            assert!(!settings.paused);
            assert_eq!(settings.retention_days, 1);
            assert_eq!(db.clips().unwrap().len(), 1);
            expected["theme"] = theme.into();
            let saved: String = db
                .conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='settings'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
                expected
            );
        }
        let before = db.settings().unwrap();
        assert!(db.set_theme("unknown").is_err());
        assert_eq!(db.settings().unwrap(), before);
        db.conn.execute_batch("CREATE TRIGGER fail_theme_save BEFORE UPDATE ON preferences WHEN NEW.key='settings' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
        assert!(db.set_theme("rose").is_err());
        assert_eq!(db.settings().unwrap(), before);
        assert_eq!(db.clips().unwrap().len(), 1);
        assert_eq!(
            db.starpet_preferences().unwrap(),
            StarpetPreferences::default()
        );
    }

    #[test]
    fn onboarding_is_upgrade_safe_persistent_and_has_no_retention_or_settings_side_effects() {
        let scratch = Scratch::new();
        let path = scratch.0.join("onboarding.sqlite3");
        let fresh = Store::open(&path).unwrap();
        assert!(!fresh.onboarding_completed().unwrap());
        drop(fresh);
        let fresh = Store::open(&path).unwrap();
        assert!(
            !fresh.onboarding_completed().unwrap(),
            "unfinished first-run status survives restart"
        );
        fresh.complete_onboarding().unwrap();
        drop(fresh);
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.onboarding_completed().unwrap());
        assert_eq!(reopened.settings().unwrap(), Settings::default());
        assert_eq!(
            reopened
                .conn
                .query_row(
                    "SELECT count(*) FROM preferences WHERE key='settings'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );

        let old_empty = store();
        old_empty
            .conn
            .execute("DELETE FROM preferences WHERE key='onboarding_v1'", [])
            .unwrap();
        let old_empty = Store::from_connection(old_empty.conn).unwrap();
        assert!(
            old_empty.onboarding_completed().unwrap(),
            "an existing empty database is still an upgrade"
        );

        let existing = store();
        existing
            .conn
            .execute("DELETE FROM preferences WHERE key='onboarding_v1'", [])
            .unwrap();
        let settings = Settings {
            paused: false,
            max_history: 50,
            retention_days: 1,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        existing
            .conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1)",
                [&json],
            )
            .unwrap();
        existing.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('引导操作不得清理这条超期片段','text',0,0)", []).unwrap();
        existing.conn.execute_batch("CREATE TRIGGER forbid_onboarding_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'onboarding must not prune'); END;").unwrap();
        assert!(existing.onboarding_completed().unwrap());
        existing.complete_onboarding().unwrap();
        assert!(existing.onboarding_completed().unwrap());
        assert_eq!(existing.clips().unwrap().len(), 1);
        assert_eq!(existing.settings().unwrap(), settings);
        assert_eq!(
            existing
                .conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='settings'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            json
        );
    }

    #[test]
    fn unlimited_history_and_new_protected_records_do_not_consume_capacity() {
        let db = store();
        assert_eq!(db.settings().unwrap().max_history, 0);
        assert_eq!(db.settings().unwrap().retention_days, 0);
        let mut bytes = 0;
        for index in 0..550 {
            let content = format!("不限历史：{index}");
            bytes += content.len() as u64;
            db.add(&content, "", None).unwrap();
        }
        assert_eq!(db.clips().unwrap().len(), 550);
        db.conn
            .execute("UPDATE clips SET updated_at=0", [])
            .unwrap();
        assert_eq!(db.prune_history().unwrap(), 0);
        assert_eq!(
            db.clips().unwrap().len(),
            550,
            "defaults do not expire old history"
        );
        let preview = db.preview_cleanup(&all_cleanup()).unwrap().preview;
        assert_eq!(preview.matched_count, 550);
        assert_eq!(preview.content_bytes, bytes);
        assert_eq!(preview.items.len(), 100);
        assert!(preview.truncated);
        assert!(preview
            .items
            .windows(2)
            .all(|items| items[0].bytes >= items[1].bytes));
        let mut settings = db.settings().unwrap();
        settings.max_history = 50;
        db.save_settings(&settings).unwrap();
        let ordinary: Vec<_> = db
            .clips()
            .unwrap()
            .into_iter()
            .map(|clip| clip.id)
            .collect();
        let keep = db
            .add_protected("新增时即永久保留", "", None, true)
            .unwrap();
        assert!(keep.keep_forever);
        assert_eq!(db.clips().unwrap().len(), 51);
        for id in ordinary {
            assert!(db.clip(id).is_ok());
        }
        settings.max_history = 0;
        settings.retention_days = 1;
        db.conn
            .execute("UPDATE clips SET updated_at=0", [])
            .unwrap();
        db.save_settings(&settings).unwrap();
        assert_eq!(
            db.clips().unwrap().len(),
            1,
            "unlimited count does not disable the chosen age policy"
        );
        assert_eq!(db.clips().unwrap()[0].id, keep.id);
    }

    #[test]
    fn permanent_protection_survives_every_mutation_and_backup_merge() {
        let mut db = store();
        let keep = db
            .add_protected("保留原创片段", "初稿", None, true)
            .unwrap();
        let pinned = db.add("星标片段", "", None).unwrap();
        db.update(pinned.id, "", None, true).unwrap();
        assert!(db.add("保留原创片段", "备注", None).unwrap().keep_forever);
        db.save_code(Some(keep.id), "代码", "println!(\"星光\");", "rust", None)
            .unwrap();
        db.update(keep.id, "取消星标仍保留", None, false).unwrap();
        assert!(db.clip(keep.id).unwrap().keep_forever);
        assert!(db.delete(keep.id).unwrap_err().contains("先取消保留"));
        db.add("可以清理的片段", "", None).unwrap();
        let mut backup: serde_json::Value = serde_json::from_str(&db.export().unwrap()).unwrap();
        for clip in backup["clips"].as_array_mut().unwrap() {
            clip["keepForever"] = false.into();
        }
        db.import(&backup.to_string()).unwrap();
        assert!(
            db.clip(keep.id).unwrap().keep_forever,
            "import cannot downgrade existing protection"
        );
        let mut restored = store();
        restored.import(&db.export().unwrap()).unwrap();
        assert!(restored
            .clips()
            .unwrap()
            .iter()
            .any(|clip| clip.keep_forever));
        db.clear_history().unwrap();
        assert_eq!(db.clips().unwrap().len(), 2);
        let preview = db.preview_cleanup(&all_cleanup()).unwrap();
        assert_eq!(preview.preview.protected_count, 2);
        assert_eq!(preview.preview.matched_count, 0);
        assert_eq!(db.execute_cleanup(preview, false).unwrap().deleted_count, 0);
        db.set_keep_forever(keep.id, false).unwrap();
        db.delete(keep.id).unwrap();
        assert!(db.clip(keep.id).is_err());
    }

    #[test]
    fn cleanup_filters_are_utf8_strict_and_combined_and_preview_is_read_only() {
        let db = store();
        db.add("星", "月", None).unwrap(); // 6 UTF-8 bytes
        db.add("abcd", "ef", None).unwrap(); // exactly the same byte threshold
        let large = db.add("银河", "夜", None).unwrap(); // 9 bytes
        let link = db.add("https://example.com", "", None).unwrap();
        let group = db.create_group("清理测试分组", "#aabbcc").unwrap();
        db.add("分组内容", "", Some(group.id)).unwrap();
        let keep = db.add_protected("保留内容", "", None, true).unwrap();
        db.update(keep.id, "", None, true).unwrap(); // overlapping protection counts once
        let pin = db.add("星标内容", "", None).unwrap();
        db.update(pin.id, "", None, true).unwrap();
        db.conn
            .execute("UPDATE clips SET updated_at=0", [])
            .unwrap();
        db.add("fresh text", "", None).unwrap();
        let changes = db.conn.total_changes();
        let mut filter = CleanupFilter {
            min_bytes: 6,
            older_than_days: 1,
            kind: "text".into(),
            include_grouped: false,
        };
        let preview = db.preview_cleanup(&filter).unwrap().preview;
        assert_eq!(db.conn.total_changes(), changes);
        assert_eq!(preview.total_count, 8);
        assert_eq!(preview.protected_count, 2);
        assert_eq!(preview.matched_count, 1);
        assert_eq!(preview.content_bytes, 9);
        assert_eq!(preview.items[0].id, large.id);
        filter.include_grouped = true;
        let grouped = db.preview_cleanup(&filter).unwrap().preview;
        assert_eq!((grouped.matched_count, grouped.content_bytes), (2, 21));
        filter.include_grouped = false;
        filter.kind = "all".into();
        let both = db.preview_cleanup(&filter).unwrap().preview;
        assert_eq!(both.matched_count, 2);
        assert!(both.items.iter().any(|item| item.id == link.id));
        assert_eq!(
            db.preview_cleanup(&all_cleanup())
                .unwrap()
                .preview
                .matched_count,
            5
        );
        filter.kind = "invalid".into();
        assert!(db.preview_cleanup(&filter).is_err());
    }

    #[test]
    fn cleanup_preview_keeps_only_compact_candidates_and_top_100_titles() {
        let mut db = store();
        for index in 0..125 {
            db.add(
                &format!("原创清理样例{index:03}{}", "星".repeat(index)),
                "",
                None,
            )
            .unwrap();
        }
        let snapshot = db.preview_cleanup(&all_cleanup()).unwrap();
        assert_eq!(snapshot.candidates.len(), 125);
        assert!(std::mem::size_of::<CleanupCandidate>() <= 48);
        assert_eq!(snapshot.preview.items.len(), 100);
        assert!(snapshot.preview.truncated);
        assert!(snapshot
            .preview
            .items
            .windows(2)
            .all(|rows| (rows[0].bytes, rows[0].id) >= (rows[1].bytes, rows[1].id)));
        let maximum = db
            .clips()
            .unwrap()
            .into_iter()
            .max_by_key(|clip| (clip_bytes(clip), clip.id))
            .unwrap();
        assert_eq!(snapshot.preview.items[0].id, maximum.id);
        assert_eq!(
            db.execute_cleanup(snapshot, false).unwrap().deleted_count,
            125
        );
    }

    #[test]
    fn stale_cleanup_checks_all_sensitive_fields_and_rolls_back() {
        for change in [
            "content='变更正文'",
            "title='变更备注'",
            "updated_at=updated_at+1",
            "group_id=1",
            "pinned=1",
            "keep_forever=1",
            "kind='code'",
            "language='rust'",
            "created_at=created_at+1",
        ] {
            let mut db = store();
            db.create_group("备选分组", "#ffffff").unwrap();
            db.add("未变化的原创片段", "", None).unwrap();
            let clip = db.add("预览中的片段", "", None).unwrap();
            let snapshot = db.preview_cleanup(&all_cleanup()).unwrap();
            db.conn
                .execute(&format!("UPDATE clips SET {change} WHERE id=?1"), [clip.id])
                .unwrap();
            assert_eq!(
                db.execute_cleanup(snapshot, false).unwrap_err(),
                STALE_CLEANUP
            );
            assert_eq!(
                db.clips().unwrap().len(),
                2,
                "{change} must not partially delete other candidates"
            );
        }
        let mut db = store();
        let old = db.add("被删除的预览项", "", None).unwrap();
        let snapshot = db.preview_cleanup(&all_cleanup()).unwrap();
        db.delete(old.id).unwrap();
        let replacement = db.add("新记录复用了最大编号", "", None).unwrap();
        assert_eq!(replacement.id, old.id);
        assert_eq!(
            db.execute_cleanup(snapshot, false).unwrap_err(),
            STALE_CLEANUP
        );
        assert!(db.clip(replacement.id).is_ok());
    }

    #[test]
    fn cleanup_does_not_delete_new_records_and_failed_delete_is_atomic() {
        let mut db = store();
        let first = db.add("预览片段一", "", None).unwrap();
        db.add("预览片段二", "", None).unwrap();
        let preview = db.preview_cleanup(&all_cleanup()).unwrap();
        let new = db.add("确认之前新收录", "", None).unwrap();
        db.mark_copied(first.id).unwrap(); // copy statistics are not a content edit
        let result = db.execute_cleanup(preview, false).unwrap();
        assert_eq!(result.deleted_count, 2);
        assert_eq!(db.clips().unwrap().len(), 1);
        assert_eq!(db.clips().unwrap()[0].id, new.id);
        db.add("另一条不能丢失的记录", "", None).unwrap();
        let preview = db.preview_cleanup(&all_cleanup()).unwrap();
        db.conn.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON clips WHEN old.content='确认之前新收录' BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
        assert!(db.execute_cleanup(preview, false).is_err());
        assert_eq!(db.clips().unwrap().len(), 2);
    }

    #[test]
    fn compaction_reports_actual_wal_file_reduction_and_handles_empty_match() {
        let scratch = Scratch::new();
        let path = scratch.0.join("cleanup.sqlite3");
        let mut db = Store::open(&path).unwrap();
        db.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.conn
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        let tx = db.conn.unchecked_transaction().unwrap();
        for index in 0..128 {
            tx.execute(
                "INSERT INTO clips(content,kind,created_at,updated_at) VALUES(?1,'text',0,0)",
                [format!("原创建库 {index} {}", "星".repeat(5500))],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let before_preview = db.preview_cleanup(&all_cleanup()).unwrap();
        assert_eq!(
            before_preview.preview.database_bytes,
            fs::metadata(&path).unwrap().len()
                + fs::metadata(format!("{}-wal", path.display()))
                    .unwrap()
                    .len()
        );
        let expected_content_bytes = before_preview.preview.content_bytes;
        let result = db.execute_cleanup(before_preview, false).unwrap();
        assert_eq!(result.deleted_count, 128);
        assert_eq!(result.content_bytes, expected_content_bytes);
        assert_eq!(
            result.released_bytes, 0,
            "deletion alone does not claim free disk space"
        );
        let empty = db.preview_cleanup(&all_cleanup()).unwrap();
        assert_eq!(empty.preview.matched_count, 0);
        assert!(empty.preview.reusable_bytes > 0);
        let result = db.execute_cleanup(empty, true).unwrap();
        assert_eq!(result.deleted_count, 0);
        assert!(result.warning.is_none());
        assert!(result.database_bytes_after < result.database_bytes_before);
        assert_eq!(
            result.released_bytes,
            result.database_bytes_before - result.database_bytes_after
        );
        assert_eq!(result.database_bytes_after, db.database_bytes().unwrap());
        assert!(db.clips().unwrap().is_empty());
    }

    #[test]
    fn checkpoint_failure_returns_committed_deletion_with_warning() {
        let scratch = Scratch::new();
        let path = scratch.0.join("cleanup.sqlite3");
        let mut db = Store::open(&path).unwrap();
        db.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.conn
            .busy_timeout(std::time::Duration::from_millis(20))
            .unwrap();
        db.add("整理失败但删除事务成功", "", None).unwrap();
        let reader = Connection::open(&path).unwrap();
        reader
            .execute_batch("BEGIN; SELECT count(*) FROM clips;")
            .unwrap();
        let preview = db.preview_cleanup(&all_cleanup()).unwrap();
        let result = db.execute_cleanup(preview, true).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert!(result.warning.as_deref().unwrap().contains("整理未完成"));
        assert!(db.clips().unwrap().is_empty());
        assert_eq!(result.database_bytes_after, db.database_bytes().unwrap());
        reader.execute_batch("ROLLBACK").unwrap();
    }

    #[test]
    fn dedup_keeps_metadata_and_group_delete_preserves_content() {
        let db = store();
        let group = db.create_group("工作", "#9977ff").unwrap();
        let first = db.add("你好，星空", "问候", Some(group.id)).unwrap();
        db.update(first.id, "问候", Some(group.id), true).unwrap();
        let again = db.add("你好，星空", "", None).unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(db.clips().unwrap().len(), 1);
        assert!(again.pinned);
        assert_eq!(again.title, "问候");
        db.delete_group(group.id).unwrap();
        assert_eq!(db.clip(first.id).unwrap().group_id, None);
        db.clear_history().unwrap();
        assert_eq!(db.clips().unwrap().len(), 1);
        db.delete(first.id).unwrap();
        assert!(db.clips().unwrap().is_empty());
    }

    #[test]
    fn retention_preserves_pins_and_groups() {
        let db = store();
        let mut settings = db.settings().unwrap();
        assert!(settings.paused);
        settings.max_history = 50;
        db.save_settings(&settings).unwrap();
        let pinned = db.add("收藏", "", None).unwrap();
        db.update(pinned.id, "", None, true).unwrap();
        let group = db.create_group("常用", "#ffffff").unwrap();
        db.add("分组常用语", "", Some(group.id)).unwrap();
        for i in 0..55 {
            db.add(&format!("第{i}项"), "", None).unwrap();
        }
        assert_eq!(db.clips().unwrap().len(), 52);
        assert!(db.clip(pinned.id).unwrap().pinned);
    }

    #[test]
    fn backup_roundtrip_merge_and_invalid_import_is_atomic() {
        let source = store();
        let group = source.create_group("链接", "#12ab34").unwrap();
        let clip = source
            .add("https://example.com", "主页", Some(group.id))
            .unwrap();
        source
            .update(clip.id, "主页", Some(group.id), true)
            .unwrap();
        let json = source.export().unwrap();
        let mut dest = store();
        assert_eq!(dest.import(&json).unwrap(), 1);
        assert_eq!(dest.import(&json).unwrap(), 0);
        assert_eq!(dest.clips().unwrap()[0].kind, "link");
        assert!(dest.clips().unwrap()[0].pinned);
        assert!(dest.settings().unwrap().paused);
        let mut invalid: serde_json::Value = serde_json::from_str(&json).unwrap();
        invalid["groups"][0]["name"] = "不应导入的分组".into();
        invalid["clips"][0]["content"] = "".into();
        assert!(dest.import(&invalid.to_string()).is_err());
        assert_eq!(dest.groups().unwrap().len(), 1);
        assert_eq!(dest.clips().unwrap().len(), 1);
    }

    #[test]
    fn code_edit_retains_identity_and_conflicts_do_not_overwrite() {
        let mut db = store();
        let code = db
            .save_code(None, "Rust 问候", "fn main() {}", "rust", None)
            .unwrap();
        assert!(code.pinned);
        assert_eq!(code.kind, "code");
        assert_eq!(code.language.as_deref(), Some("rust"));
        db.mark_copied(code.id).unwrap();
        let edited = db
            .save_code(
                Some(code.id),
                "入口",
                "fn main() { println!(\"hi\"); }",
                "rust",
                None,
            )
            .unwrap();
        assert_eq!(edited.id, code.id);
        assert_eq!(edited.copy_count, 1);
        let other = db.add("other", "不能覆盖", None).unwrap();
        assert!(db
            .save_code(Some(code.id), "错误", "other", "rust", None)
            .is_err());
        assert_eq!(db.clip(other.id).unwrap().title, "不能覆盖");
        assert_eq!(db.clip(code.id).unwrap().content, edited.content);
        let backup = db.export().unwrap();
        db.clear_history().unwrap();
        assert_eq!(db.import(&backup).unwrap(), 1);
        assert_eq!(db.clip(code.id).unwrap().language.as_deref(), Some("rust"));
    }

    #[test]
    fn backup_limits_roundtrip_and_reject_overflow() {
        let source = store();
        source.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000)
            INSERT INTO groups(name,color) SELECT 'group-'||i,'#ffffff' FROM n;
            WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<10000)
            INSERT INTO clips(content,title,kind,pinned,created_at,updated_at) SELECT 'clip-'||i,'','text',1,0,0 FROM n;").unwrap();
        let json = source.export().unwrap();
        let mut destination = store();
        assert_eq!(destination.import(&json).unwrap(), MAX_BACKUP_CLIPS);
        assert_eq!(destination.groups().unwrap().len(), MAX_BACKUP_GROUPS);
        assert!(check_backup_limits(MAX_BACKUP_BYTES, MAX_BACKUP_CLIPS, MAX_BACKUP_GROUPS).is_ok());
        assert!(check_backup_limits(MAX_BACKUP_BYTES + 1, 0, 0).is_err());
        let extra = source.add("overflow", "", None).unwrap();
        assert!(source.export().is_err());
        source.delete(extra.id).unwrap();
        source.create_group("overflow", "#ffffff").unwrap();
        assert!(source.export().is_err());
    }

    #[test]
    fn stale_preference_draft_preserves_newer_capture_choice_atomically() {
        let db = store();
        let mut opened = db.settings().unwrap();
        opened.paused = false;
        db.save_settings(&opened).unwrap();
        opened = db.settings().unwrap();
        let observed_revision = opened.pause_revision;
        let mut draft = opened.clone();
        draft.layout = "list".into();
        draft.always_on_top = true;

        // The tray pauses after the UI read, before its save reaches the store.
        let mut tray = opened;
        tray.paused = true;
        db.save_settings(&tray).unwrap();
        let saved = db
            .save_settings_with_pause_guard(&draft, Some(observed_revision))
            .unwrap();
        assert!(
            saved.paused,
            "an unrelated preference save cannot resume capture"
        );
        assert_eq!(saved.layout, "list");
        assert!(saved.always_on_top);
        assert_eq!(saved.pause_revision, observed_revision + 1);
        assert_eq!(db.settings().unwrap(), saved);

        // A deliberate resume based on the current saved pause still works.
        draft = saved;
        draft.paused = false;
        let resumed = db
            .save_settings_with_pause_guard(&draft, Some(draft.pause_revision))
            .unwrap();
        assert!(!resumed.paused);
        assert_eq!(resumed.pause_revision, draft.pause_revision + 1);
        assert_eq!(db.settings().unwrap(), resumed);

        // Invalid unrelated fields cannot partially commit either preference.
        draft.layout = "invalid".into();
        draft.paused = true;
        assert!(db
            .save_settings_with_pause_guard(&draft, Some(resumed.pause_revision))
            .is_err());
        assert_eq!(db.settings().unwrap(), resumed);
    }

    #[test]
    fn pause_revision_blocks_aba_and_is_owned_by_the_store() {
        let db = store();
        let opened = db.settings().unwrap();
        assert!(opened.paused);
        assert_eq!(opened.pause_revision, 0);
        let mut stale_resume = opened.clone();
        stale_resume.paused = false;
        stale_resume.layout = "list".into();

        // Internal / tray writes must increment even when no guard was supplied.
        let mut tray = opened.clone();
        tray.paused = false;
        tray.pause_revision = u64::MAX;
        db.save_settings(&tray).unwrap();
        assert_eq!(db.settings().unwrap().pause_revision, 1);
        tray.paused = true;
        db.save_settings(&tray).unwrap();
        let paused = db.settings().unwrap();
        assert!(paused.paused);
        assert_eq!(paused.pause_revision, 2);

        // The boolean is back to its old value, but the old resume is stale.
        let saved = db
            .save_settings_with_pause_guard(&stale_resume, Some(opened.pause_revision))
            .unwrap();
        assert!(saved.paused);
        assert_eq!(saved.pause_revision, 2);
        assert_eq!(saved.layout, "list");

        // A current explicit resume works; an arbitrary payload version cannot win.
        let mut fresh_resume = saved.clone();
        fresh_resume.paused = false;
        fresh_resume.pause_revision = u64::MAX;
        let resumed = db
            .save_settings_with_pause_guard(&fresh_resume, Some(saved.pause_revision))
            .unwrap();
        assert!(!resumed.paused);
        assert_eq!(resumed.pause_revision, 3);
        assert_eq!(db.settings().unwrap(), resumed);
        db.save_settings(&fresh_resume).unwrap();
        assert_eq!(
            db.settings().unwrap().pause_revision,
            3,
            "unchanged capture does not advance the version"
        );
        assert_eq!(db.set_theme("forest").unwrap().pause_revision, 3);
    }

    #[test]
    fn legacy_settings_and_backup_keep_their_meaning() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE clips (
            id INTEGER PRIMARY KEY, content TEXT NOT NULL UNIQUE, title TEXT NOT NULL DEFAULT '',
            kind TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, group_id INTEGER,
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, copy_count INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            INSERT INTO clips(content,kind,pinned,created_at,updated_at) VALUES('旧版原创片段','text',1,0,0);").unwrap();
        let legacy = r#"{"paused":true,"theme":"light","layout":"list","maxHistory":500,"shortcut":"CommandOrControl+Shift+V"}"#;
        conn.execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [legacy],
        )
        .unwrap();
        let db = Store::from_connection(conn).unwrap();
        let settings = db.settings().unwrap();
        assert_eq!(
            settings.pause_revision, 0,
            "old stored settings need no migration write"
        );
        assert!(!settings.always_on_top);
        assert!(settings.show_startup);
        assert_eq!(settings.sticker_pack, "auto");
        assert_eq!(
            settings.max_history, 500,
            "legacy configured limits remain unchanged"
        );
        assert_eq!(settings.retention_days, 0);
        assert_eq!(settings.layout, "list");
        assert_eq!(db.clips().unwrap()[0].language, None);
        assert!(!db.clips().unwrap()[0].keep_forever);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='settings'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            legacy
        );
        let mut old_backup: serde_json::Value =
            serde_json::from_str(&db.export().unwrap()).unwrap();
        old_backup["clips"][0]
            .as_object_mut()
            .unwrap()
            .remove("language");
        old_backup["clips"][0]
            .as_object_mut()
            .unwrap()
            .remove("keepForever");
        let mut destination = store();
        let mut updated = settings.clone();
        updated.layout = "horizontal".into();
        updated.always_on_top = true;
        updated.retention_days = 7;
        updated.show_startup = false;
        updated.sticker_pack = "cats".into();
        for theme in ["forest", "rose", "white"] {
            updated.theme = theme.into();
            destination.save_settings(&updated).unwrap();
            assert_eq!(destination.settings().unwrap(), updated);
        }
        destination.save_settings(&updated).unwrap();
        assert_eq!(destination.import(&old_backup.to_string()).unwrap(), 1);
        assert_eq!(destination.settings().unwrap(), updated);
        assert!(destination.clips().unwrap()[0].pinned);
        assert!(!destination.clips().unwrap()[0].keep_forever);
        updated.retention_days = 2;
        assert!(destination.save_settings(&updated).is_err());
        assert_eq!(destination.settings().unwrap().retention_days, 7);
    }

    #[test]
    fn age_cleanup_preserves_protected_items_and_recapture_renews_history() {
        let db = store();
        let mut settings = db.settings().unwrap();
        settings.retention_days = 7;
        db.save_settings(&settings).unwrap();
        let expired = db.add("过期片段", "", None).unwrap();
        let renewed = db.add("再次收录", "", None).unwrap();
        let pinned = db.add("重要星标", "", None).unwrap();
        db.update(pinned.id, "", None, true).unwrap();
        let group = db.create_group("常用片段", "#aabbcc").unwrap();
        let grouped = db.add("分组保护", "", Some(group.id)).unwrap();
        db.conn
            .execute("UPDATE clips SET created_at=0,updated_at=0", [])
            .unwrap();
        let fresh = db.add("再次收录", "", None).unwrap();
        assert_eq!(fresh.id, renewed.id);
        assert_eq!(fresh.created_at, 0);
        assert!(fresh.updated_at > 0);
        assert!(db.clip(expired.id).is_err());
        assert!(db.clip(pinned.id).is_ok());
        assert!(db.clip(grouped.id).is_ok());
        let boundary = fresh.updated_at - 7 * 86_400_000;
        db.conn
            .execute(
                "UPDATE clips SET updated_at=?1 WHERE id=?2",
                params![boundary, fresh.id],
            )
            .unwrap();
        assert_eq!(
            Store::prune_with_settings(&db.conn, &settings, fresh.updated_at).unwrap(),
            0
        );
        assert_eq!(
            Store::prune_with_settings(&db.conn, &settings, fresh.updated_at + 1).unwrap(),
            1
        );
        assert_eq!(db.clips().unwrap().len(), 2);
    }

    #[test]
    fn age_and_capacity_apply_together_at_startup_and_settings_are_atomic() {
        let db = store();
        let mut settings = db.settings().unwrap();
        settings.retention_days = 7;
        settings.max_history = 50;
        db.save_settings(&settings).unwrap();
        let timestamp = now();
        // Seed history directly so one maintenance pass exercises both limits together.
        let tx = db.conn.unchecked_transaction().unwrap();
        for index in 0..60 {
            let updated = if index < 5 { 0 } else { timestamp + index };
            tx.execute(
                "INSERT INTO clips(content,kind,created_at,updated_at) VALUES(?1,'text',0,?2)",
                params![format!("容量与时间测试 {index}"), updated],
            )
            .unwrap();
        }
        tx.execute("INSERT INTO clips(content,kind,pinned,created_at,updated_at) VALUES('受保护星标','text',1,0,0)", []).unwrap();
        tx.commit().unwrap();
        let reopened = Store::from_connection(db.conn).unwrap();
        assert_eq!(reopened.clips().unwrap().len(), 51);
        assert_eq!(reopened.prune_history().unwrap(), 0);
        assert!(reopened
            .clips()
            .unwrap()
            .iter()
            .any(|clip| clip.content == "容量与时间测试 10"));
        assert!(!reopened
            .clips()
            .unwrap()
            .iter()
            .any(|clip| clip.content == "容量与时间测试 9"));
        reopened
            .conn
            .execute("UPDATE clips SET updated_at=0 WHERE pinned=0", [])
            .unwrap();
        reopened.conn.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'test cleanup failure'); END;").unwrap();
        let mut changed = settings.clone();
        changed.retention_days = 1;
        changed.always_on_top = true;
        assert!(reopened.save_settings(&changed).is_err());
        assert_eq!(reopened.settings().unwrap(), settings);
        assert_eq!(reopened.clips().unwrap().len(), 51);
    }

    #[test]
    fn image_migration_preserves_legacy_fields_and_separates_text_identity() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;
            CREATE TABLE groups(id INTEGER PRIMARY KEY,name TEXT NOT NULL UNIQUE,color TEXT NOT NULL);
            INSERT INTO groups VALUES(7,'原创旧分组','#abcdef');
            CREATE TABLE clips(id INTEGER PRIMARY KEY,content TEXT NOT NULL UNIQUE,title TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,keep_forever INTEGER NOT NULL DEFAULT 0,
                group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,copy_count INTEGER NOT NULL DEFAULT 0,language TEXT);
            INSERT INTO clips VALUES(11,'fn original() {}','旧代码','code',1,1,7,12,34,9,'rust');").unwrap();
        let db = Store::from_connection(conn).unwrap();
        let legacy = db.clip(11).unwrap();
        assert_eq!(
            (legacy.created_at, legacy.updated_at, legacy.copy_count),
            (12, 34, 9)
        );
        assert_eq!(legacy.group_id, Some(7));
        assert!(legacy.pinned && legacy.keep_forever);
        assert_eq!(legacy.language.as_deref(), Some("rust"));
        assert_eq!(legacy.title, "旧代码");
        assert!(legacy.image.is_none());
        let old_backup: serde_json::Value = serde_json::from_str(&db.export().unwrap()).unwrap();
        assert_eq!(old_backup["version"], 4);
        assert!(old_backup.get("images").is_none());

        let mut budget_clip = legacy.clone();
        budget_clip.image = Some(ImageMetadata {
            width: 4000,
            height: 4000,
            bytes: 100,
        });
        assert!(check_backup_image_limits(&vec![budget_clip.clone(); 16]).is_ok());
        assert!(check_backup_image_limits(&vec![budget_clip; 17]).is_err());

        let pixels = image_fixture(50);
        let text = db.add(pixels.digest(), "恰好相同的普通文本", None).unwrap();
        let picture = db.add_image(&pixels, "图片", Some(7), true).unwrap();
        assert_ne!(text.id, picture.id);
        db.update(picture.id, "图片标题", Some(7), true).unwrap();
        db.mark_copied(picture.id).unwrap();
        db.conn
            .execute("UPDATE clips SET updated_at=0 WHERE id=?1", [picture.id])
            .unwrap();
        let repeated = db.add_image(&pixels, "", None, false).unwrap();
        assert_eq!(picture.id, repeated.id);
        assert!(repeated.updated_at > 0);
        assert!(repeated.pinned && repeated.keep_forever);
        assert_eq!(repeated.copy_count, 1);
        assert_eq!(repeated.group_id, Some(7));
        assert_eq!(repeated.title, "图片标题");
        assert_eq!(db.add(pixels.digest(), "", None).unwrap().id, text.id);
        assert_eq!(db.clip(text.id).unwrap().kind, "text");
        assert!(db
            .save_code(Some(picture.id), "不应转换", "hello", "plaintext", None)
            .is_err());
        let list_json = serde_json::to_value(db.clips().unwrap()).unwrap();
        let listed = list_json
            .as_array()
            .unwrap()
            .iter()
            .find(|clip| clip["id"] == picture.id)
            .unwrap();
        assert_eq!(listed["image"]["bytes"], pixels.png.len());
        assert!(listed.get("png").is_none());
        db.delete_group(7).unwrap();
        assert!(db.clip(11).unwrap().group_id.is_none());
        assert!(db.clip(picture.id).unwrap().group_id.is_none());
        let reopened = Store::from_connection(db.conn).unwrap();
        assert_eq!(reopened.image_png(picture.id, false).unwrap(), pixels.png);
    }

    #[test]
    fn image_backup_roundtrip_validation_and_failure_are_atomic() {
        let source = store();
        let group = source.create_group("原创像素", "#abcdef").unwrap();
        let pixels = image_fixture(91);
        let picture = source
            .add_image(&pixels, "透明像素测试", Some(group.id), true)
            .unwrap();
        source
            .update(picture.id, "透明像素测试", Some(group.id), true)
            .unwrap();
        source.add(pixels.digest(), "相同标识的文本", None).unwrap();
        let json = source.export().unwrap();
        let backup: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(backup["version"], 4);
        assert_eq!(
            STANDARD
                .decode(backup["images"][0]["png"].as_str().unwrap())
                .unwrap(),
            pixels.png
        );
        let mut destination = store();
        assert_eq!(destination.import(&json).unwrap(), 2);
        assert_eq!(destination.import(&json).unwrap(), 0);
        let imported = destination
            .clips()
            .unwrap()
            .into_iter()
            .find(|clip| clip.kind == "image")
            .unwrap();
        assert_eq!(
            destination.image_png(imported.id, false).unwrap(),
            pixels.png
        );
        assert_eq!(
            destination.image_png(imported.id, true).unwrap(),
            pixels.thumbnail
        );
        assert!(imported.pinned && imported.keep_forever && imported.group_id.is_some());
        assert!(destination.settings().unwrap().paused);
        let mut unprotected = backup.clone();
        for clip in unprotected["clips"].as_array_mut().unwrap() {
            clip["keepForever"] = false.into();
            clip["pinned"] = false.into();
        }
        destination.import(&unprotected.to_string()).unwrap();
        assert!(destination.clip(imported.id).unwrap().keep_forever);
        assert!(destination.clip(imported.id).unwrap().pinned);

        let picture_index = backup["clips"]
            .as_array()
            .unwrap()
            .iter()
            .position(|clip| clip["kind"] == "image")
            .unwrap();
        for fault in 0..9 {
            let mut broken = backup.clone();
            broken["groups"][0]["name"] = "不应留下的分组".into();
            match fault {
                0 => broken["images"] = serde_json::json!([]),
                1 => broken["images"][0]["png"] = "not-base64".into(),
                2 => broken["clips"][picture_index]["image"]["width"] = 99.into(),
                3 => broken["clips"][picture_index]["image"]["bytes"] = 1.into(),
                4 => broken["clips"][picture_index]["content"] = "sha256:forged".into(),
                5 => broken["version"] = 1.into(),
                6 => broken["images"][0]["clipId"] = 9999.into(),
                7 => {
                    let duplicate = broken["images"][0].clone();
                    broken["images"].as_array_mut().unwrap().push(duplicate);
                }
                _ => {
                    let mut png = pixels.png.clone();
                    png[29] ^= 1;
                    broken["images"][0]["png"] = STANDARD.encode(png).into();
                }
            }
            assert!(
                destination.import(&broken.to_string()).is_err(),
                "fault {fault}"
            );
            assert_eq!(destination.clips().unwrap().len(), 2);
            assert_eq!(destination.groups().unwrap().len(), 1);
        }
        let mut failing = store();
        failing.conn.execute_batch("CREATE TRIGGER refuse_image BEFORE INSERT ON clip_images BEGIN SELECT RAISE(ABORT, 'fixture image write failure'); END;").unwrap();
        assert!(failing.import(&json).is_err());
        assert!(failing.clips().unwrap().is_empty());
        assert!(failing.groups().unwrap().is_empty());
        assert!(failing.add_image(&pixels, "写入失败", None, false).is_err());
        assert!(failing.clips().unwrap().is_empty());
    }

    #[test]
    fn image_cleanup_bytes_protection_and_cascades_cover_all_delete_paths() {
        let mut db = store();
        let pixels = image_fixture(12);
        let picture = db.add_image(&pixels, "星图", None, false).unwrap();
        let expected = pixels.png.len() as u64 + "星图".len() as u64;
        let mut filter = all_cleanup();
        filter.kind = "image".into();
        filter.min_bytes = expected;
        assert_eq!(
            db.preview_cleanup(&filter).unwrap().preview.matched_count,
            0
        );
        filter.min_bytes -= 1;
        let preview = db.preview_cleanup(&filter).unwrap();
        assert_eq!(preview.preview.content_bytes, expected);
        assert_eq!(preview.preview.items[0].bytes, expected);
        db.set_keep_forever(picture.id, true).unwrap();
        assert!(db.execute_cleanup(preview, false).is_err());
        assert!(db.delete(picture.id).is_err());
        db.clear_history().unwrap();
        assert!(db.image_png(picture.id, false).is_ok());
        db.set_keep_forever(picture.id, false).unwrap();
        let preview = db.preview_cleanup(&filter).unwrap();
        let next = db
            .add_image(&image_fixture(13), "预览之后新图片", None, false)
            .unwrap();
        let result = db.execute_cleanup(preview, false).unwrap();
        assert_eq!(result.deleted_count, 1);
        assert_eq!(result.content_bytes, expected);
        assert!(db.image_png(picture.id, false).is_err());
        assert!(db.image_png(next.id, false).is_ok());
        db.delete(next.id).unwrap();
        assert_eq!(
            db.conn
                .query_row("SELECT count(*) FROM clip_images", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );

        let group = db.create_group("保护图片", "#123abc").unwrap();
        let protected = db
            .add_image(&image_fixture(20), "永久", None, true)
            .unwrap();
        let pinned = db
            .add_image(&image_fixture(21), "星标", None, false)
            .unwrap();
        db.update(pinned.id, "星标", None, true).unwrap();
        let grouped = db
            .add_image(&image_fixture(22), "分组", Some(group.id), false)
            .unwrap();
        let expired = db
            .add_image(&image_fixture(23), "超时", None, false)
            .unwrap();
        db.conn
            .execute("UPDATE clips SET updated_at=0", [])
            .unwrap();
        let settings = Settings {
            retention_days: 1,
            ..Settings::default()
        };
        db.save_settings(&settings).unwrap();
        assert!(db.image_png(expired.id, false).is_err());
        for id in [protected.id, pinned.id, grouped.id] {
            assert!(db.image_png(id, false).is_ok());
        }
        db.clear_history().unwrap();
        assert!(db.image_png(grouped.id, false).is_err());
        for id in [protected.id, pinned.id] {
            assert!(db.image_png(id, false).is_ok());
        }
        let mut settings = settings;
        settings.retention_days = 0;
        settings.max_history = 50;
        db.save_settings(&settings).unwrap();
        let old = db
            .add_image(&image_fixture(24), "容量淘汰", None, false)
            .unwrap();
        for index in 0..50 {
            db.add(&format!("原创容量片段 {index}"), "", None).unwrap();
        }
        assert!(db.image_png(old.id, false).is_err());
        assert_eq!(
            db.conn
                .query_row("SELECT count(*) FROM clip_images", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
    }

    #[test]
    fn wal_snapshot_preserves_original_and_thumbnail_without_external_assets() {
        let scratch = Scratch::new();
        let path = scratch.0.join("images.sqlite3");
        let snapshot = scratch.0.join("snapshot.sqlite3");
        let db = Store::open(&path).unwrap();
        db.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        db.conn
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        let pixels =
            images::prepare_rgba(640, 160, &[90, 120, 210, 120].repeat(640 * 160)).unwrap();
        let clip = db
            .add_image(&pixels, "原创半透明横幅", None, false)
            .unwrap();
        db.snapshot(&snapshot).unwrap();
        Store::validate_database(&snapshot).unwrap();
        db.delete(clip.id).unwrap();
        let copy = Store::open(&snapshot).unwrap();
        assert_eq!(copy.image_png(clip.id, false).unwrap(), pixels.png);
        assert_eq!(copy.image_png(clip.id, true).unwrap(), pixels.thumbnail);
        assert_eq!(
            copy.clip(clip.id).unwrap().image.as_ref(),
            Some(pixels.metadata())
        );
        assert_eq!(
            images::decode_png(&copy.image_png(clip.id, true).unwrap())
                .unwrap()
                .0,
            320
        );
        assert!(db.clips().unwrap().is_empty());
    }

    #[test]
    fn unicode_and_limits_are_checked_without_mutation() {
        let db = store();
        assert!(db.add("  ", "", None).is_err());
        assert!(db
            .add(&"x".repeat(MAX_CONTENT_BYTES + 1), "", None)
            .is_err());
        assert!(db.create_group("星", "#星星").is_err());
        assert_eq!(classify("#a1b2c3"), "color");
        assert_eq!(classify("https://example.com\nhello"), "text");
        assert!(db.clips().unwrap().is_empty());
    }
    #[test]
    fn ui_language_defaults_to_system_persists_and_survives_theme_writes() {
        let scratch = Scratch::new();
        let path = scratch.0.join("language.sqlite3");
        let db = Store::open(&path).unwrap();
        assert_eq!(db.settings().unwrap().ui_language, "system");

        // A preference written before the language field existed keeps working.
        let legacy = r#"{"paused":true,"theme":"dark","layout":"grid","maxHistory":0,"shortcut":"CommandOrControl+Shift+V"}"#;
        db.conn
            .execute(
                "INSERT INTO preferences(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [legacy],
            )
            .unwrap();
        assert_eq!(db.settings().unwrap().ui_language, "system");

        let mut settings = db.settings().unwrap();
        settings.ui_language = "en-US".into();
        db.save_settings(&settings).unwrap();
        assert_eq!(db.settings().unwrap().ui_language, "en-US");

        // The immediate theme write must preserve other stored preferences.
        db.set_theme("forest").unwrap();
        let saved = db.settings().unwrap();
        assert_eq!(saved.ui_language, "en-US");
        assert_eq!(saved.theme, "forest");

        let unsupported = Settings {
            ui_language: "fr".into(),
            ..Settings::default()
        };
        assert!(unsupported.validate().is_err());
    }

    /// A real directory tree with original names: Chinese, spaces, special
    /// characters, and two same-named files in different folders.
    fn file_tree() -> Scratch {
        let scratch = Scratch::new();
        let root = scratch.0.clone();
        fs::create_dir_all(root.join("第一 层/子目录 & 符号")).unwrap();
        fs::create_dir_all(root.join("第二层")).unwrap();
        fs::write(
            root.join("第一 层/原创 笔记 #1.txt"),
            b"original fixture one",
        )
        .unwrap();
        fs::write(
            root.join("第一 层/子目录 & 符号/同名.txt"),
            b"original fixture two",
        )
        .unwrap();
        fs::write(root.join("第二层/同名.txt"), b"original fixture three").unwrap();
        scratch
    }

    fn entries_of(scratch: &Scratch, relative: &[&str]) -> Vec<FileEntry> {
        crate::clipboard_files::normalize(
            relative
                .iter()
                .map(|item| scratch.0.join(item).to_string_lossy().into_owned()),
        )
        .unwrap()
    }

    #[test]
    fn file_records_keep_order_kinds_and_a_separate_identity_space() {
        let scratch = file_tree();
        let entries = entries_of(
            &scratch,
            &[
                "第一 层/原创 笔记 #1.txt",
                "第一 层/子目录 & 符号",
                "第二层/同名.txt",
            ],
        );
        assert_eq!(entries[0].entry_kind, FileEntryKind::File);
        assert_eq!(entries[1].entry_kind, FileEntryKind::Directory);
        assert_eq!(entries[2].entry_kind, FileEntryKind::File);

        let db = store();
        let clip = db.add_files(&entries, "", None, false).unwrap();
        assert_eq!(clip.kind, "file");
        assert_eq!(clip.entries.as_deref(), Some(entries.as_slice()));
        assert_eq!(clip.content, crate::clipboard_files::content(&entries));

        // The same joined paths pasted as text are a different record.
        let text = db.add(&clip.content, "", None).unwrap();
        assert_ne!(text.id, clip.id);
        assert_eq!(text.kind, "text");

        // Copying the same list again refreshes one record instead of adding one.
        let again = db.add_files(&entries, "", None, false).unwrap();
        assert_eq!(again.id, clip.id);
        assert_eq!(db.clips().unwrap().len(), 2);

        // The original files are only referenced, never rewritten.
        let source = scratch.0.join("第一 层/原创 笔记 #1.txt");
        assert_eq!(fs::read(&source).unwrap(), b"original fixture one");

        // Deleting the record cascades to its path rows.
        db.delete(clip.id).unwrap();
        let left: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM clip_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
        assert_eq!(fs::read(&source).unwrap(), b"original fixture one");
    }

    #[test]
    fn file_records_are_filterable_searchable_and_never_ai_eligible() {
        let scratch = file_tree();
        let entries = entries_of(
            &scratch,
            &["第一 层/子目录 & 符号/同名.txt", "第二层/同名.txt"],
        );
        let db = store();
        let clip = db.add_files(&entries, "", None, false).unwrap();

        let page = db
            .history_page(&HistoryQuery {
                filter: "file".into(),
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(page.history.total, 1);
        assert_eq!(page.history.counts.get("file"), Some(&1));
        assert_eq!(page.clips[0].entries.as_deref(), Some(entries.as_slice()));

        // Search covers both the file name and the folder path.
        for needle in ["同名", "子目录 & 符号", "第二层"] {
            let found = db
                .history_page(&HistoryQuery {
                    query: needle.into(),
                    ..HistoryQuery::default()
                })
                .unwrap();
            assert_eq!(found.history.total, 1, "search missed {needle}");
        }

        // A file list is never offered to text/image-only creation features.
        let eligible = db
            .history_page(&HistoryQuery {
                ai_eligible: true,
                ..HistoryQuery::default()
            })
            .unwrap();
        assert_eq!(eligible.history.total, 0);
        assert!(!db.has_pending_ai_history().unwrap());
        assert!(!db.clip(clip.id).unwrap().is_private);

        // Nor can it be turned into a code snippet.
        assert!(db
            .save_code(Some(clip.id), "", "原创代码", "rust", None)
            .is_err());
    }

    #[test]
    fn file_backup_roundtrip_keeps_paths_and_rejects_inconsistent_lists() {
        let scratch = file_tree();
        let entries = entries_of(&scratch, &["第一 层/原创 笔记 #1.txt", "第二层/同名.txt"]);
        let db = store();
        let clip = db.add_files(&entries, "", None, true).unwrap();
        let json = db.export().unwrap();

        let mut restored = store();
        assert_eq!(restored.import(&json).unwrap(), 1);
        let imported = restored.clip(clip.id).unwrap();
        assert_eq!(imported.kind, "file");
        assert_eq!(imported.entries.as_deref(), Some(entries.as_slice()));

        // Re-importing the same backup merges into one record, not two.
        restored.import(&json).unwrap();
        assert_eq!(restored.clips().unwrap().len(), 1);
        let rows: i64 = restored
            .conn
            .query_row("SELECT COUNT(*) FROM clip_files", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rows, 2);

        // A list that disagrees with the record content is refused.
        let mut tampered: serde_json::Value = serde_json::from_str(&json).unwrap();
        tampered["clips"][0]["content"] = "/original/mismatch".into();
        assert!(restored.import(&tampered.to_string()).is_err());

        // A relative path in a backup is refused.
        let mut relative: serde_json::Value = serde_json::from_str(&json).unwrap();
        relative["clips"][0]["entries"][0]["path"] = "relative/path".into();
        assert!(restored.import(&relative.to_string()).is_err());

        // A file record without its list is refused.
        let mut missing: serde_json::Value = serde_json::from_str(&json).unwrap();
        missing["clips"][0]
            .as_object_mut()
            .unwrap()
            .remove("entries");
        assert!(restored.import(&missing.to_string()).is_err());

        // Paths that no longer exist still restore: nothing is scanned.
        let mut elsewhere = store();
        let mut gone: serde_json::Value = serde_json::from_str(&json).unwrap();
        gone["clips"][0]["entries"][0]["path"] = "/original/gone.txt".into();
        let second = gone["clips"][0]["entries"][1]["path"]
            .as_str()
            .unwrap()
            .to_owned();
        gone["clips"][0]["content"] =
            serde_json::Value::String(format!("/original/gone.txt\n{second}"));
        assert_eq!(elsewhere.import(&gone.to_string()).unwrap(), 1);
    }

    #[test]
    fn file_schema_upgrade_keeps_old_rows_and_separates_indexes() {
        let scratch = Scratch::new();
        let path = scratch.0.join("starclip.db");
        let legacy = {
            let db = Store::open(&path).unwrap();
            db.add("原创旧记录", "", None).unwrap();
            db
        };
        drop(legacy);
        // Simulate the previous schema: no file table, older version marker.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "DROP TABLE clip_files; DROP INDEX clips_file_content; PRAGMA user_version=2;",
            )
            .unwrap();
        }
        let upgraded = Store::open(&path).unwrap();
        assert_eq!(upgraded.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(upgraded.clips().unwrap().len(), 1);
        assert!(upgraded
            .conn
            .prepare("SELECT clip_id,position,path,entry_kind FROM clip_files LIMIT 0")
            .is_ok());
        // The text index no longer covers file rows, so both can share content.
        let entries = entries_of(&scratch, &["第一 层/原创 笔记 #1.txt"]);
        let file = upgraded.add_files(&entries, "", None, false).unwrap();
        let text = upgraded.add(&file.content, "", None).unwrap();
        assert_ne!(file.id, text.id);
        assert!(upgraded.add_files(&entries, "", None, false).is_ok());
    }

    #[test]
    fn file_records_require_paths_and_stay_out_of_text_creation() {
        let db = store();
        assert!(db.add_files(&[], "", None, false).is_err());
        let scratch = file_tree();
        let entries = entries_of(&scratch, &["第二层/同名.txt"]);
        let clip = db.add_files(&entries, "", None, false).unwrap();
        // Editing keeps the reference list; only title/group/pin change.
        db.update(clip.id, "文件标题", None, true).unwrap();
        let updated = db.clip(clip.id).unwrap();
        assert_eq!(updated.entries.as_deref(), Some(entries.as_slice()));
        assert_eq!(updated.title, "文件标题");
        assert!(updated.pinned);
    }
}
