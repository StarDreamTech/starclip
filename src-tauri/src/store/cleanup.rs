use super::*;

impl Store {
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

    pub(super) fn database_bytes(&self) -> Result<u64> {
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

    pub(super) fn compact_database(&self) -> Result<()> {
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

    pub(super) fn prune_with_settings(
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
}
