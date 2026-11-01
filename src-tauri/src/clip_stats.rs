//! 星迹统计：本机剪贴活动计数。
//!
//! 记录表只保存「15 分钟时段 × 类型 × 事件」的次数与大小合计，不保存正文、
//! 标题、域名或记录 ID，也不进入 JSON 备份或可选功能统计。历史去重会覆盖
//! `updated_at`、清理会删除记录，所以每日次数不能从 `clips` 推算，需在采集
//! 同一事务内追加计数。表随数据库迁移与恢复点一起保存，旧版本忽略这张表。
use crate::store::{sql_error, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::BTreeMap;

/// 15 分钟能精确对齐所有现行时区（含 +5:30、+5:45）。
pub const BUCKET_MS: i64 = 15 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const RETENTION_DAYS: i64 = 400;
pub const MAX_RANGE_DAYS: u32 = 366;
pub const TOP_HOSTS: usize = 8;
const MAX_OFFSET_MINUTES: i32 = 14 * 60;
pub const KINDS: [&str; 6] = ["text", "link", "code", "color", "image", "file"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// 首次进入历史的内容。
    New,
    /// 已在历史中的内容被再次复制（去重后只更新时间）。
    Repeat,
    /// 从星笺内复制回系统剪贴板。
    Reuse,
}

impl Event {
    fn as_str(self) -> &'static str {
        match self {
            Event::New => "new",
            Event::Repeat => "repeat",
            Event::Reuse => "reuse",
        }
    }
}

pub fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clip_activity (
            bucket INTEGER NOT NULL, kind TEXT NOT NULL,
            event TEXT NOT NULL CHECK(event IN ('new','repeat','reuse')),
            count INTEGER NOT NULL DEFAULT 0 CHECK(count>=0),
            size INTEGER NOT NULL DEFAULT 0 CHECK(size>=0),
            PRIMARY KEY(bucket,kind,event)) WITHOUT ROWID;",
    )
    .map_err(sql_error)
}

/// `size` 对文字类为字符数，对图片为 PNG 字节数；复制回不计大小。
pub fn record(conn: &Connection, at_ms: i64, kind: &str, event: Event, size: u64) -> Result<()> {
    let kind = if KINDS.contains(&kind) { kind } else { "text" };
    let bucket = at_ms.div_euclid(BUCKET_MS);
    let size = i64::try_from(size).unwrap_or(i64::MAX);
    conn.execute(
        "INSERT INTO clip_activity(bucket,kind,event,count,size) VALUES(?1,?2,?3,1,?4)
         ON CONFLICT(bucket,kind,event) DO UPDATE SET count=count+1,size=MIN(size+excluded.size,9223372036854775807)",
        params![bucket, kind, event.as_str(), size],
    )
    .map_err(sql_error)?;
    conn.execute(
        "DELETE FROM clip_activity WHERE bucket<?1",
        [(at_ms - RETENTION_DAYS * DAY_MS).div_euclid(BUCKET_MS)],
    )
    .map_err(sql_error)?;
    Ok(())
}

pub fn clear(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM clip_activity", [])
        .map_err(sql_error)?;
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindActivity {
    pub new: u64,
    pub repeat: u64,
    pub reuse: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    /// 本地日期序号：自 1970-01-01（本地）起的天数。
    pub day: i64,
    pub captures: BTreeMap<String, u64>,
    pub new: u64,
    pub repeat: u64,
    pub reuse: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCount {
    pub host: String,
    pub count: u64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub total: u64,
    pub kinds: BTreeMap<String, u64>,
    pub pinned: u64,
    pub kept: u64,
    pub grouped: u64,
    pub private: u64,
    pub reused_clips: u64,
    pub copy_total: u64,
    pub image_bytes: u64,
    pub oldest_created_at: Option<i64>,
    /// 仅统计非隐私记录的来源域名。
    pub top_hosts: Vec<HostCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipStats {
    pub days: u32,
    pub utc_offset_minutes: i32,
    pub first_day: i64,
    pub last_day: i64,
    /// 最早一条计数所在时段的开始时间（毫秒），保留期内无计数时为空。
    pub recording_since: Option<i64>,
    /// 只含有活动的日期，按日期升序。
    pub daily: Vec<DailyActivity>,
    /// 采集次数（首次 + 再次复制）按本地小时、星期（0 = 周日）分布。
    pub hours: [u64; 24],
    pub weekdays: [u64; 7],
    pub kinds: BTreeMap<String, KindActivity>,
    pub library: LibraryStats,
}

pub fn validate_range(days: u32, utc_offset_minutes: i32) -> Result<()> {
    if !(1..=MAX_RANGE_DAYS).contains(&days) {
        return Err(format!("统计范围需为 1–{MAX_RANGE_DAYS} 天。"));
    }
    if utc_offset_minutes.abs() > MAX_OFFSET_MINUTES || utc_offset_minutes % 15 != 0 {
        return Err("时区偏移无效。".into());
    }
    Ok(())
}

/// 统计截至 `now_ms` 所在本地日的最近 `days` 天。时区偏移由界面按当前时区提供；
/// 跨夏令时切换的旧日期可能整体偏一小时。
pub fn activity(
    conn: &Connection,
    now_ms: i64,
    days: u32,
    utc_offset_minutes: i32,
    private_predicate: &str,
) -> Result<ClipStats> {
    validate_range(days, utc_offset_minutes)?;
    let offset_ms = i64::from(utc_offset_minutes) * 60_000;
    let last_day = (now_ms + offset_ms).div_euclid(DAY_MS);
    let first_day = last_day - i64::from(days) + 1;
    let from_bucket = (first_day * DAY_MS - offset_ms).div_euclid(BUCKET_MS);
    let to_bucket = ((last_day + 1) * DAY_MS - offset_ms).div_euclid(BUCKET_MS);
    let mut daily: BTreeMap<i64, DailyActivity> = BTreeMap::new();
    let mut hours = [0u64; 24];
    let mut weekdays = [0u64; 7];
    let mut kinds: BTreeMap<String, KindActivity> = KINDS
        .iter()
        .map(|kind| ((*kind).to_string(), KindActivity::default()))
        .collect();
    let mut statement = conn
        .prepare(
            "SELECT bucket,kind,event,count,size FROM clip_activity WHERE bucket>=?1 AND bucket<?2",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(params![from_bucket, to_bucket], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(sql_error)?;
    for row in rows {
        let (bucket, kind, event, count, size) = row.map_err(sql_error)?;
        let (count, size) = (count.max(0) as u64, size.max(0) as u64);
        let local = bucket * BUCKET_MS + offset_ms;
        let day = local.div_euclid(DAY_MS);
        let entry = daily.entry(day).or_insert_with(|| DailyActivity {
            day,
            captures: BTreeMap::new(),
            new: 0,
            repeat: 0,
            reuse: 0,
        });
        let totals = kinds.entry(kind.clone()).or_default();
        match event.as_str() {
            "new" | "repeat" => {
                if event == "new" {
                    entry.new += count;
                    totals.new += count;
                } else {
                    entry.repeat += count;
                    totals.repeat += count;
                }
                totals.size = totals.size.saturating_add(size);
                *entry.captures.entry(kind).or_default() += count;
                hours[(local.rem_euclid(DAY_MS) / 3_600_000) as usize] += count;
                weekdays[(day + 4).rem_euclid(7) as usize] += count;
            }
            _ => {
                entry.reuse += count;
                totals.reuse += count;
            }
        }
    }
    let recording_since = conn
        .query_row("SELECT MIN(bucket) FROM clip_activity", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .map_err(sql_error)?
        .map(|bucket| bucket * BUCKET_MS);
    Ok(ClipStats {
        days,
        utc_offset_minutes,
        first_day,
        last_day,
        recording_since,
        daily: daily.into_values().collect(),
        hours,
        weekdays,
        kinds,
        library: library(conn, private_predicate)?,
    })
}

fn library(conn: &Connection, private_predicate: &str) -> Result<LibraryStats> {
    let count = |sql: &str| -> Result<u64> {
        conn.query_row(sql, [], |row| row.get::<_, i64>(0))
            .map(|value| value.max(0) as u64)
            .map_err(sql_error)
    };
    let mut stats = LibraryStats::default();
    let mut statement = conn
        .prepare("SELECT kind,COUNT(*) FROM clips GROUP BY kind")
        .map_err(sql_error)?;
    for row in statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(sql_error)?
    {
        let (kind, value) = row.map_err(sql_error)?;
        stats.kinds.insert(kind, value.max(0) as u64);
    }
    stats.total = stats.kinds.values().sum();
    stats.pinned = count("SELECT COUNT(*) FROM clips WHERE pinned=1")?;
    stats.kept = count("SELECT COUNT(*) FROM clips WHERE keep_forever=1")?;
    stats.grouped = count("SELECT COUNT(*) FROM clips WHERE group_id IS NOT NULL")?;
    stats.private = count(&format!(
        "SELECT COUNT(*) FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id WHERE {private_predicate}"
    ))?;
    stats.reused_clips = count("SELECT COUNT(*) FROM clips WHERE copy_count>0")?;
    stats.copy_total = count("SELECT COALESCE(SUM(MAX(copy_count,0)),0) FROM clips")?;
    stats.image_bytes = count("SELECT COALESCE(SUM(bytes),0) FROM clip_images")?;
    stats.oldest_created_at = conn
        .query_row("SELECT MIN(created_at) FROM clips", [], |row| row.get(0))
        .map_err(sql_error)?;
    let mut statement = conn
        .prepare(&format!(
            "SELECT h.value,COUNT(DISTINCT c.id) AS n FROM clips c JOIN clip_privacy p ON p.clip_id=c.id,
             json_each(p.metadata,'$.linkHosts') h WHERE h.type='text' AND NOT {private_predicate}
             GROUP BY h.value ORDER BY n DESC,h.value LIMIT {TOP_HOSTS}"
        ))
        .map_err(sql_error)?;
    stats.top_hosts = statement
        .query_map([], |row| {
            Ok(HostCount {
                host: row.get(0)?,
                count: row.get::<_, i64>(1)?.max(0) as u64,
            })
        })
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql_error)?;
    Ok(stats)
}
