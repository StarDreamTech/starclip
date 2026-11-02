use super::*;

const MAX_CACHE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct HistoryGlobalStamp {
    changes: u64,
    data_version: i64,
    schema_version: i64,
}

#[derive(Clone)]
pub(super) struct HistoryGlobal {
    pub counts: BTreeMap<String, u64>,
    pub group_counts: BTreeMap<i64, u64>,
    pub sources: Vec<String>,
    pub hosts: Vec<String>,
    pub tags: Vec<String>,
}

impl HistoryGlobal {
    fn estimated_bytes(&self) -> usize {
        // An approximate retention budget, not a process RSS limit. Do not keep
        // unusually large facets in addition to the complete response itself.
        let mut bytes =
            std::mem::size_of::<Self>().saturating_add(self.group_counts.len().saturating_mul(48));
        for key in self.counts.keys() {
            bytes = bytes.saturating_add(key.len()).saturating_add(64);
        }
        for value in self.sources.iter().chain(&self.hosts).chain(&self.tags) {
            bytes = bytes
                .saturating_add(value.len())
                .saturating_add(std::mem::size_of::<String>());
        }
        bytes
    }
}

pub(super) struct CachedHistoryGlobal {
    stamp: HistoryGlobalStamp,
    value: HistoryGlobal,
}

impl Store {
    fn history_global_stamp(&self) -> Result<HistoryGlobalStamp> {
        Ok(HistoryGlobalStamp {
            changes: self.conn.total_changes(),
            data_version: self
                .conn
                .pragma_query_value(None, "data_version", |row| row.get(0))
                .map_err(sql_error)?,
            schema_version: self
                .conn
                .pragma_query_value(None, "schema_version", |row| row.get(0))
                .map_err(sql_error)?,
        })
    }

    pub(super) fn history_global(&self) -> Result<HistoryGlobal> {
        // Autocommit can remain true while a live Rows cursor pins a read
        // snapshot. Neither explicit transactions nor those older snapshots
        // may consume or publish the shared committed-data cache.
        if !self.conn.is_autocommit() || self.conn.is_busy() {
            self.history_global_cache.borrow_mut().take();
            return self.query_history_global();
        }
        let before = self.history_global_stamp()?;
        {
            let mut cached = self.history_global_cache.borrow_mut();
            if let Some(entry) = cached.as_ref().filter(|entry| entry.stamp == before) {
                return Ok(entry.value.clone());
            }
            cached.take();
        }
        // All uncached aggregates use one read snapshot. Compare versions only
        // after leaving it: an external commit during the read must not label
        // the old snapshot with the new database version.
        let tx = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Deferred,
        )
        .map_err(sql_error)?;
        let value = self.query_history_global()?;
        tx.commit().map_err(sql_error)?;
        if self.history_global_stamp()? == before && value.estimated_bytes() <= MAX_CACHE_BYTES {
            *self.history_global_cache.borrow_mut() = Some(CachedHistoryGlobal {
                stamp: before,
                value: value.clone(),
            });
        }
        Ok(value)
    }

    #[cfg(test)]
    pub(super) fn history_global_query_count(&self) -> u64 {
        self.history_global_queries.get()
    }

    fn query_history_global(&self) -> Result<HistoryGlobal> {
        let from = "FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id";
        #[cfg(test)]
        self.history_global_queries
            .set(self.history_global_queries.get().saturating_add(1));
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
        Ok(HistoryGlobal {
            counts,
            group_counts,
            sources: facets(&["linkSources", "originSources"])?,
            hosts: facets(&["linkHosts", "originHosts"])?,
            tags: facets(&["manualTags", "ruleTags", "appTags"])?,
        })
    }
}
