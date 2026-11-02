use super::*;

/// Validate portable absolute references without looking at the local filesystem.
fn absolute_path(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    let normalized = path.replace('\\', "/");
    let drive = |value: &str| {
        let bytes = value.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
    };
    let unc = |value: &str| {
        let parts: Vec<_> = value.split('/').take(2).collect();
        parts.len() == 2
            && parts
                .iter()
                .all(|part| !part.is_empty() && !matches!(*part, "." | ".." | "?"))
    };
    if drive(&normalized) {
        return true;
    }
    if let Some(rest) = normalized.strip_prefix("//?/") {
        if rest
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("UNC/"))
        {
            return unc(&rest[4..]);
        }
        // Allow Windows extended drive roots, not device / GLOBALROOT namespaces.
        return drive(rest);
    }
    normalized.strip_prefix("//").is_some_and(unc)
}

pub(super) fn file_content(entries: &[FileEntry]) -> Result<String> {
    if entries.is_empty() || entries.len() > MAX_FILES {
        return Err("文件记录需包含 1–100 项路径。".into());
    }
    let mut seen = HashSet::new();
    let mut bytes = 0usize;
    for entry in entries {
        let path = entry.path.as_str();
        if path.is_empty()
            || path.chars().any(|c| matches!(c, '\0' | '\r' | '\n'))
            || !absolute_path(path)
            || !seen.insert(path)
        {
            return Err("文件路径无效、重复或不是绝对路径。".into());
        }
        bytes = bytes.saturating_add(path.len() + 1);
    }
    if bytes > MAX_CONTENT_BYTES {
        return Err("文件列表超过 256 KiB。".into());
    }
    Ok(entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>()
        .join("\n"))
}

pub(super) fn unknown_entries(content: &str) -> Result<Vec<FileEntry>> {
    let entries: Vec<_> = content
        .split('\n')
        .map(|path| FileEntry {
            path: path.into(),
            entry_kind: FileEntryKind::Unknown,
        })
        .collect();
    if file_content(&entries)? != content {
        return Err("文件记录与路径列表不一致。".into());
    }
    Ok(entries)
}

impl Store {
    pub(super) fn put_file_entries(
        conn: &Connection,
        id: i64,
        entries: &[FileEntry],
    ) -> Result<()> {
        file_content(entries)?;
        conn.execute("DELETE FROM clip_files WHERE clip_id=?1", [id])
            .map_err(sql_error)?;
        let mut statement = conn
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
        Ok(())
    }

    pub(super) fn fill_entries(conn: &Connection, clips: &mut [Clip]) -> Result<()> {
        // Bound SQL parameters even for full JSON export.
        for chunk in clips.chunks_mut(500) {
            let ids: Vec<_> = chunk
                .iter()
                .filter(|clip| clip.kind == "file")
                .map(|clip| clip.id)
                .collect();
            if ids.is_empty() {
                continue;
            }
            let placeholders = vec!["?"; ids.len()].join(",");
            let mut statement = conn.prepare(&format!("SELECT clip_id,position,path,entry_kind FROM clip_files WHERE clip_id IN ({placeholders}) ORDER BY clip_id,position")).map_err(sql_error)?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(ids.iter()), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, usize>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(sql_error)?;
            let mut grouped: HashMap<i64, Vec<FileEntry>> = HashMap::new();
            for row in rows {
                let (id, position, path, kind) = row.map_err(sql_error)?;
                let entries = grouped.entry(id).or_default();
                if position != entries.len() {
                    return Err("文件路径顺序损坏。".into());
                }
                let entry_kind = match kind.as_str() {
                    "file" => FileEntryKind::File,
                    "directory" => FileEntryKind::Directory,
                    "unknown" => FileEntryKind::Unknown,
                    _ => return Err("文件路径类型损坏。".into()),
                };
                entries.push(FileEntry { path, entry_kind });
            }
            for clip in chunk.iter_mut().filter(|clip| clip.kind == "file") {
                let entries = grouped.remove(&clip.id).ok_or("文件记录缺少路径列表。")?;
                if file_content(&entries)? != clip.content {
                    return Err("文件记录与路径列表不一致。".into());
                }
                clip.entries = Some(entries);
            }
        }
        Ok(())
    }
}
