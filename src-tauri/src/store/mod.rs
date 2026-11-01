use crate::appearance::{self, Appearance};
use crate::clip_stats::{self, ClipStats};
use crate::images::{self, ImageMetadata, PreparedImage};
use crate::privacy::{self, CaptureContext, Metadata, PrivacyRescan, PrivacyRules};
use crate::starpet_growth::{GrowthChange, GrowthState, MAX_REVISION};
use crate::telemetry::TelemetryPreferences;
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
pub const SCHEMA_VERSION: u32 = 2;
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
    /// 复制时记录的来源应用，同时出现在 `tags` 中。
    #[serde(default)]
    pub app_tags: Vec<String>,
    /// 浏览器提供的来源页面（无查询与片段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_url: Option<String>,
    /// 采集时附带了 HTML 格式；复制回去会同时恢复格式，可选择只复制纯文本。
    /// JSON 备份不含 HTML，完整数据目录迁移保留。
    #[serde(default, skip_deserializing)]
    pub rich: bool,
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
    /// 复制记录后收起主窗口，回到原应用即可粘贴。
    #[serde(default = "default_hide_after_copy")]
    pub hide_after_copy: bool,
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
fn default_hide_after_copy() -> bool {
    true
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
            hide_after_copy: default_hide_after_copy(),
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

/// Picks the work-area edge closest to a dropped pet window, if any is within
/// `threshold`, and returns that edge with the window position flush against it.
/// The other axis is only clamped, so the pet stays where it was dropped along the edge.
#[cfg(any(feature = "desktop", test))]
pub(crate) fn starpet_dock_position(
    position: (f64, f64),
    size: (f64, f64),
    work_area: (f64, f64, f64, f64),
    threshold: f64,
) -> Option<(&'static str, f64, f64)> {
    let (left, top, width, height) = work_area;
    let right = (left + width - size.0).max(left);
    let bottom = (top + height - size.1).max(top);
    let x = position.0.clamp(left, right);
    let y = position.1.clamp(top, bottom);
    // Side walls win ties: clinging sideways reads better than lying in a corner.
    [
        ("left", position.0 - left, left, y),
        ("right", right - position.0, right, y),
        ("bottom", bottom - position.1, x, bottom),
        ("top", position.1 - top, x, top),
    ]
    .into_iter()
    .filter(|(_, gap, _, _)| *gap <= threshold)
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(edge, _, x, y)| (edge, x, y))
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

// A legacy image capability marker is not a privacy match; image pixels
// remain unscanned and can never become AI-eligible via this predicate.
const PRIVATE_PREDICATE: &str = "(COALESCE(json_extract(p.metadata,'$.privacyManual'),0)=1 OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.privacyReasons') WHERE c.kind!='image' OR value!='图片尚未进行文字识别'))";
/// 采集时随正文附带的数据。
#[derive(Clone, Copy)]
enum CaptureExtra<'a> {
    Html(Option<&'a str>),
    Files,
}
const CLIP_SELECT: &str = "SELECT c.id,c.content,c.title,c.kind,c.pinned,c.group_id,c.created_at,c.updated_at,c.copy_count,c.language,c.keep_forever,i.width,i.height,i.bytes,p.metadata,EXISTS(SELECT 1 FROM clip_rich r WHERE r.clip_id=c.id) FROM clips c LEFT JOIN clip_images i ON i.clip_id=c.id LEFT JOIN clip_privacy p ON p.clip_id=c.id";

pub struct Store {
    conn: Connection,
}

pub(crate) fn sql_error(error: rusqlite::Error) -> String {
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
        keep_forever: row.get(10)?,
        privacy_manual: metadata.privacy_manual,
        is_private: metadata.is_private(),
        tags: metadata.tags(),
        manual_tags: metadata.manual_tags.clone(),
        privacy_reasons: metadata.privacy_reasons,
        scan_version: metadata.scan_version,
        link_sources: metadata.link_sources,
        link_hosts: metadata.link_hosts,
        app_tags: metadata.app_tags,
        origin_url: (!metadata.origin_url.is_empty()).then_some(metadata.origin_url),
        rich: row.get(15)?,
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
/// 富文本只作为文字记录的附加格式保存，随记录删除。
pub const MAX_RICH_BYTES: usize = 256 * 1024;
pub const MAX_FILES: usize = 64;
fn ensure_rich_table(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS clip_rich (clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE, html TEXT NOT NULL);")
        .map_err(sql_error)
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

mod backup;
mod cleanup;
mod clips;
mod history;
mod preferences;
mod privacy_rules;
mod schema;

#[cfg(test)]
mod tests;
