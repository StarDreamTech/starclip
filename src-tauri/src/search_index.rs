//! 历史搜索的 FTS5 trigram 索引。
//!
//! 索引只作为候选预筛：命中后仍由 `starclip_contains` 做与界面一致的 Unicode 小写包含判断，
//! 所以搜索结果与全表扫描相同，只是不再逐行比较全部历史。trigram 需要至少 3 个字符，
//! 更短的关键词（如两个汉字）继续走原来的扫描。
//!
//! 表是 contentless（不保存第二份正文），并开启 FTS5 `secure-delete`，删除记录时立即从索引中
//! 移除对应词元，配合数据库 `secure_delete` 不在索引里残留已删内容。
use crate::store::{sql_error, Result};
use rusqlite::Connection;

/// 索引的正文与搜索谓词保持一致：标题 + 换行 + 文字正文（图片只索引标题）。
const BODY: &str = "title || char(10) || CASE WHEN kind='image' THEN '' ELSE content END";
const NEW_BODY: &str =
    "new.title || char(10) || CASE WHEN new.kind='image' THEN '' ELSE new.content END";
pub const MIN_QUERY_CHARS: usize = 3;

fn exists(conn: &Connection, kind: &str, name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
        [kind, name],
        |row| row.get(0),
    )
    .map_err(sql_error)
}

/// 建表与触发器；索引缺失或触发器曾随旧版重建 clips 表丢失时，从现有记录完整重建。
pub fn ensure(conn: &Connection) -> Result<()> {
    let table = exists(conn, "table", "clips_fts")?;
    let triggers = exists(conn, "trigger", "clips_fts_insert")?
        && exists(conn, "trigger", "clips_fts_update")?
        && exists(conn, "trigger", "clips_fts_delete")?;
    if table && triggers {
        return Ok(());
    }
    conn.execute_batch(&format!(
        "DROP TABLE IF EXISTS clips_fts;
        CREATE VIRTUAL TABLE clips_fts USING fts5(body, tokenize='trigram', content='', contentless_delete=1);
        INSERT INTO clips_fts(clips_fts, rank) VALUES('secure-delete', 1);
        DROP TRIGGER IF EXISTS clips_fts_insert;
        DROP TRIGGER IF EXISTS clips_fts_update;
        DROP TRIGGER IF EXISTS clips_fts_delete;
        CREATE TRIGGER clips_fts_insert AFTER INSERT ON clips BEGIN
            INSERT INTO clips_fts(rowid, body) VALUES(new.id, {new_body});
        END;
        CREATE TRIGGER clips_fts_update AFTER UPDATE OF title, content, kind ON clips BEGIN
            DELETE FROM clips_fts WHERE rowid=old.id;
            INSERT INTO clips_fts(rowid, body) VALUES(new.id, {new_body});
        END;
        CREATE TRIGGER clips_fts_delete AFTER DELETE ON clips BEGIN
            DELETE FROM clips_fts WHERE rowid=old.id;
        END;
        INSERT INTO clips_fts(rowid, body) SELECT id, {BODY} FROM clips;",
        new_body = NEW_BODY,
    ))
    .map_err(sql_error)
}

/// 关键词足够长时返回 FTS 短语查询，否则返回 None 以回退到全表扫描。
pub fn match_phrase(lowercase_query: &str) -> Option<String> {
    (lowercase_query.chars().count() >= MIN_QUERY_CHARS)
        .then(|| format!("\"{}\"", lowercase_query.replace('"', "\"\"")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phrase_needs_three_characters_and_escapes_quotes() {
        assert_eq!(match_phrase("星笺"), None);
        assert_eq!(match_phrase("原创星").as_deref(), Some("\"原创星\""));
        assert_eq!(match_phrase("a\"b").as_deref(), Some("\"a\"\"b\""));
    }
}
