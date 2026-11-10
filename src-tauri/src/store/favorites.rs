use super::*;

// Optional collection preferences, like the activity/rich-text extensions. Older
// clients can continue to use the core schema and leave these preferences intact.
pub(super) fn ensure(conn: &Connection) -> Result<()> {
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clip_favorites')", [], |row|row.get(0)).map_err(sql_error)?;
    let transaction = if conn.is_autocommit() {
        Some(conn.unchecked_transaction().map_err(sql_error)?)
    } else {
        None
    };
    conn.execute_batch("CREATE TABLE IF NOT EXISTS clip_favorites (
        clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,
        on_top INTEGER NOT NULL DEFAULT 0 CHECK(on_top IN (0,1)),
        position INTEGER NOT NULL CHECK(position>=0)
    );
    CREATE INDEX IF NOT EXISTS clip_favorites_position ON clip_favorites(on_top DESC,position,clip_id);
    CREATE INDEX IF NOT EXISTS clip_favorites_last_position ON clip_favorites(position);
    CREATE TRIGGER IF NOT EXISTS clip_favorites_insert AFTER INSERT ON clips WHEN NEW.pinned=1 BEGIN
        INSERT OR IGNORE INTO clip_favorites(clip_id,position) VALUES(NEW.id,MIN(COALESCE((SELECT MAX(position) FROM clip_favorites),0)+1024,9000000000000000));
    END;
    CREATE TRIGGER IF NOT EXISTS clip_favorites_pin AFTER UPDATE OF pinned ON clips WHEN NEW.pinned=1 AND OLD.pinned=0 BEGIN
        INSERT OR IGNORE INTO clip_favorites(clip_id,position) VALUES(NEW.id,MIN(COALESCE((SELECT MAX(position) FROM clip_favorites),0)+1024,9000000000000000));
    END;
    CREATE TRIGGER IF NOT EXISTS clip_favorites_unpin AFTER UPDATE OF pinned ON clips WHEN NEW.pinned=0 AND OLD.pinned=1 BEGIN
        DELETE FROM clip_favorites WHERE clip_id=NEW.id;
    END;")
        .map_err(sql_error)?;
    if !exists {
        conn.execute("INSERT INTO clip_favorites(clip_id,position) SELECT id,ROW_NUMBER() OVER(ORDER BY updated_at DESC,id DESC)*1024 FROM clips WHERE pinned=1", []).map_err(sql_error)?;
    }
    if let Some(transaction) = transaction {
        transaction.commit().map_err(sql_error)?;
    }
    Ok(())
}

impl Store {
    pub fn set_favorite_top(&self, id: i64, on_top: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE clip_favorites SET on_top=?2 WHERE clip_id=?1 AND EXISTS(SELECT 1 FROM clips WHERE id=?1 AND pinned=1)",
            params![id, on_top],
        ).map_err(sql_error)?;
        if changed == 0 {
            return Err("请先将记录加入星标收藏。".into());
        }
        Ok(())
    }

    pub fn move_favorite(&self, id: i64, target: Option<i64>, before: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let top: bool = tx
            .query_row(
                "SELECT on_top FROM clip_favorites WHERE clip_id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|_| "请先将记录加入星标收藏。".to_string())?;
        let mut ordered = {
            let mut statement = tx.prepare("SELECT clip_id,position FROM clip_favorites WHERE on_top=?1 ORDER BY position,clip_id DESC").map_err(sql_error)?;
            let rows = statement
                .query_map([top], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(sql_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?
        };
        if Some(id) == target {
            return Ok(());
        }
        let source = ordered
            .iter()
            .position(|item| item.0 == id)
            .ok_or("收藏记录已改变，请刷新后重试。")?;
        ordered.remove(source);
        let index = match target {
            Some(target) => {
                ordered
                    .iter()
                    .position(|item| item.0 == target)
                    .ok_or("置顶项与普通收藏分别排序，请选择同一区域的记录。")?
                    + usize::from(!before)
            }
            None => {
                if before {
                    0
                } else {
                    ordered.len()
                }
            }
        };
        let lower = index.checked_sub(1).map(|i| ordered[i].1).unwrap_or(0);
        let upper = ordered.get(index).map(|item| item.1);
        let position = match upper {
            Some(upper) if upper > lower + 1 => Some(lower + (upper - lower) / 2),
            None => lower
                .checked_add(1024)
                .filter(|value| *value <= MAX_FAVORITE_POSITION),
            _ => None,
        };
        let position = match position {
            Some(position) => position,
            None => {
                let mut statement = tx
                    .prepare("UPDATE clip_favorites SET position=?2 WHERE clip_id=?1")
                    .map_err(sql_error)?;
                for (index, item) in ordered.iter().enumerate() {
                    statement
                        .execute(params![item.0, (index as i64 + 1) * 1024])
                        .map_err(sql_error)?;
                }
                (index as i64) * 1024 + 512
            }
        };
        tx.execute(
            "UPDATE clip_favorites SET position=?2 WHERE clip_id=?1",
            params![id, position],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }
}

pub(super) const MAX_FAVORITE_POSITION: i64 = 9_000_000_000_000_000;

#[cfg(test)]
mod tests {
    use super::*;

    fn favorite(store: &Store, text: &str) -> i64 {
        let clip = store.add(text, text, None).unwrap();
        store.update(clip.id, text, None, true).unwrap();
        clip.id
    }

    #[test]
    fn favorite_sort_and_top_apply_before_pagination_and_move_across_pages() {
        let store = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let mut ids = Vec::new();
        for index in 0..121 {
            let id = favorite(&store, &format!("Original favorite {index}"));
            store
                .conn
                .execute(
                    "UPDATE clips SET created_at=?2,updated_at=?2,copy_count=?3 WHERE id=?1",
                    params![id, index, index % 10],
                )
                .unwrap();
            ids.push(id);
        }
        store.set_favorite_top(ids[0], true).unwrap();
        for sort in ["updated", "created", "copies", "manual"] {
            let result = store
                .history_page(&HistoryQuery {
                    filter: "pinned".into(),
                    sort: sort.into(),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(result.history.total, 121);
            assert_eq!(result.clips.len(), 50);
            assert_eq!(result.clips[0].id, ids[0]);
            assert!(result.clips[0].on_top);
            if sort == "copies" {
                assert_eq!(result.clips[1].copy_count, 9);
            }
        }
        store.move_favorite(ids[120], None, true).unwrap();
        let query = HistoryQuery {
            filter: "pinned".into(),
            sort: "manual".into(),
            ..Default::default()
        };
        assert_eq!(store.history_page(&query).unwrap().clips[1].id, ids[120]);
        store.move_favorite(ids[120], None, false).unwrap();
        let last = store
            .history_page(&HistoryQuery { page: 3, ..query })
            .unwrap();
        assert_eq!(last.clips.last().unwrap().id, ids[120]);
        assert_eq!(store.clip(ids[120]).unwrap().updated_at, 120);
        assert_eq!(
            store.history_page(&HistoryQuery::default()).unwrap().clips[0].id,
            ids[120]
        );
        assert!(store.move_favorite(ids[120], Some(ids[0]), true).is_err());
    }

    #[test]
    fn favorite_order_rebalances_sparse_positions_and_roundtrips_without_overwriting_existing_order(
    ) {
        let source = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        let ids: Vec<_> = (0..3)
            .map(|i| favorite(&source, &format!("Original manual position {i}")))
            .collect();
        let mut expected = ids.clone();
        for _ in 0..40 {
            let id = expected.pop().unwrap();
            expected.insert(0, id);
            source.move_favorite(id, None, true).unwrap();
        }
        let query = HistoryQuery {
            filter: "pinned".into(),
            sort: "manual".into(),
            ..Default::default()
        };
        assert_eq!(
            source
                .history_page(&query)
                .unwrap()
                .clips
                .iter()
                .map(|clip| clip.id)
                .collect::<Vec<_>>(),
            expected
        );
        source.set_favorite_top(expected[0], true).unwrap();
        let json = source.export().unwrap();
        let mut target = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        assert_eq!(target.import(&json).unwrap(), 3);
        let contents = |store: &Store| {
            store
                .history_page(&query)
                .unwrap()
                .clips
                .into_iter()
                .map(|clip| (clip.content, clip.on_top))
                .collect::<Vec<_>>()
        };
        assert_eq!(contents(&source), contents(&target));
        let last = target
            .history_page(&query)
            .unwrap()
            .clips
            .last()
            .unwrap()
            .id;
        target.move_favorite(last, None, true).unwrap();
        let current = contents(&target);
        assert_eq!(target.import(&json).unwrap(), 0);
        assert_eq!(contents(&target), current);
        let plain = target.add("Original unstarred fixture", "", None).unwrap();
        assert!(target.set_favorite_top(plain.id, true).is_err());
        target.update(last, "", None, false).unwrap();
        assert!(!target.clip(last).unwrap().on_top);
        assert!(target.clip(last).unwrap().favorite_position.is_none());
        let mut bad: serde_json::Value = serde_json::from_str(&json).unwrap();
        bad["clips"][0]["favoritePosition"] = serde_json::json!(-1);
        let before = target.export().unwrap();
        assert!(target.import(&bad.to_string()).is_err());
        assert_eq!(target.clips().unwrap().len(), 4);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&before).unwrap()["clips"],
            serde_json::from_str::<serde_json::Value>(&target.export().unwrap()).unwrap()["clips"]
        );
    }

    #[test]
    fn existing_schema_five_opens_without_optional_favorite_preferences_and_reopens_with_them() {
        let root = super::super::tests::Scratch::new();
        let path = root.0.join("starclip.sqlite3");
        let store = Store::open(&path).unwrap();
        let id = favorite(&store, "Original existing schema five favorite");
        let identity = store.clip(id).unwrap().record_identity;
        store.conn.execute_batch("DROP TRIGGER clip_favorites_insert; DROP TRIGGER clip_favorites_pin; DROP TRIGGER clip_favorites_unpin; DROP TABLE clip_favorites;").unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.clip(id).unwrap().record_identity, identity);
        store.set_favorite_top(id, true).unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        assert!(store.clip(id).unwrap().on_top);
        assert_eq!(store.clip(id).unwrap().record_identity, identity);
        store.set_favorite_top(id, false).unwrap();
        store
            .conn
            .execute(
                "UPDATE clip_favorites SET position=?1",
                [MAX_FAVORITE_POSITION],
            )
            .unwrap();
        favorite(&store, "Original large rank fixture");
        let mut restored = Store::from_connection(Connection::open_in_memory().unwrap()).unwrap();
        restored.import(&store.export().unwrap()).unwrap();
        let query = HistoryQuery {
            filter: "pinned".into(),
            sort: "manual".into(),
            ..Default::default()
        };
        let contents = |store: &Store| {
            store
                .history_page(&query)
                .unwrap()
                .clips
                .into_iter()
                .map(|clip| clip.content)
                .collect::<Vec<_>>()
        };
        assert_eq!(contents(&store), contents(&restored));
    }
}
