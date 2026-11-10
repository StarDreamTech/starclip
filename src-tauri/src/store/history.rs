use super::*;

impl Store {
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
        let mut group_filter = None;
        match query.filter.as_str() {
            "all" => {}
            "pinned" => predicates.push("c.pinned=1".into()),
            "kept" => predicates.push("c.keep_forever=1".into()),
            "link" => predicates
                .push("(c.kind='link' OR json_array_length(p.metadata,'$.linkHosts')>0)".into()),
            "image" => predicates.push("c.kind IN ('image','bundle')".into()),
            "text" | "code" | "color" | "file" => {
                predicates.push("c.kind=?".into());
                values.push(query.filter.clone().into());
            }
            group if group.starts_with("group-") => {
                let id = group[6..].parse::<i64>().map_err(|_| "分组编号无效。")?;
                group_filter = Some(id);
                predicates.push("c.group_id=?".into());
                values.push(id.into());
            }
            _ => return Err("历史分类无效。".into()),
        }
        let search = query.query.trim().to_lowercase();
        if !search.is_empty() {
            let tags = if query.ai_eligible {
                " || COALESCE(json_extract(p.metadata,'$.manualTags'),'') || COALESCE(json_extract(p.metadata,'$.ruleTags'),'') || COALESCE(json_extract(p.metadata,'$.appTags'),'')"
            } else {
                ""
            };
            // 标签只在 AI 授权列表里参与搜索，不在索引中；其余情况先用 trigram 索引缩小候选。
            if let Some(phrase) = (!query.ai_eligible)
                .then(|| crate::search_index::match_phrase(&search))
                .flatten()
            {
                predicates
                    .push("c.id IN (SELECT rowid FROM clips_fts WHERE clips_fts MATCH ?)".into());
                values.push(phrase.into());
            }
            predicates.push(format!("starclip_contains(c.title || char(10) || CASE WHEN c.kind='image' THEN '' ELSE c.content END{tags},?)"));
            values.push(search.into());
        }
        if !query.sources.is_empty() {
            let mut sources = vec![];
            for source in &query.sources {
                let ((path, origin), value) = match source.strip_prefix("host:") {
                    Some(host) => (("linkHosts", "originHosts"), host),
                    None => (("linkSources", "originSources"), source.as_str()),
                };
                sources.push(format!(
                    "EXISTS(SELECT 1 FROM json_each(p.metadata,'$.{path}') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.{origin}') WHERE value=?)"
                ));
                values.extend([value.to_owned().into(), value.to_owned().into()]);
            }
            predicates.push(format!("({})", sources.join(" OR ")));
        }
        if !query.tag.is_empty() {
            predicates.push("(EXISTS(SELECT 1 FROM json_each(p.metadata,'$.manualTags') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.ruleTags') WHERE value=?) OR EXISTS(SELECT 1 FROM json_each(p.metadata,'$.appTags') WHERE value=?))".into());
            values.extend([
                query.tag.clone().into(),
                query.tag.clone().into(),
                query.tag.clone().into(),
            ]);
        }
        let private = PRIVATE_PREDICATE;
        let pending = format!(
            "(c.kind IN ('image','bundle') OR COALESCE(json_extract(p.metadata,'$.scanVersion'),0)!={revision})"
        );
        match query.privacy.as_str() {
            "all" => {}
            "private" => predicates.push(private.into()),
            "pending" => predicates.push(pending.clone()),
            "ordinary" => predicates.push(format!("NOT {private} AND NOT {pending}")),
            _ => return Err("隐私筛选无效。".into()),
        }
        if query.ai_eligible {
            predicates.push(format!(
                "c.kind NOT IN ('image','file','bundle') AND NOT {private} AND NOT {pending}"
            ));
        }
        let from = "FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id";
        let where_sql = predicates.join(" AND ");
        let global = self.history_global()?;
        let total: u64 = if query.query.trim().is_empty()
            && query.sources.is_empty()
            && query.tag.is_empty()
            && query.privacy == "all"
            && !query.ai_eligible
        {
            match group_filter {
                Some(id) => global.group_counts.get(&id).copied().unwrap_or(0),
                None => global.counts.get(&query.filter).copied().unwrap_or(0),
            }
        } else {
            self.conn
                .query_row(
                    &format!("SELECT COUNT(*) {from} WHERE {where_sql}"),
                    rusqlite::params_from_iter(values.iter()),
                    |row| row.get(0),
                )
                .map_err(sql_error)?
        };
        let last = total
            .div_ceil(u64::from(HISTORY_PAGE_SIZE))
            .max(1)
            .min(u64::from(u32::MAX)) as u32;
        let page = query.page.max(1).min(last);
        let offset = u64::from(page - 1) * u64::from(HISTORY_PAGE_SIZE);
        let order = if query.filter == "pinned" {
            match query.sort.as_str() {
                "updated" => "f.on_top DESC,c.updated_at DESC,c.id DESC",
                "created" => "f.on_top DESC,c.created_at DESC,c.id DESC",
                "copies" => "f.on_top DESC,c.copy_count DESC,c.updated_at DESC,c.id DESC",
                "manual" => "f.on_top DESC,f.position ASC,c.id DESC",
                _ => return Err("未知的收藏排序方式。".into()),
            }
        } else {
            "c.updated_at DESC,c.id DESC"
        };
        let mut statement = self.conn.prepare(&format!("{CLIP_SELECT} WHERE {where_sql} ORDER BY {order} LIMIT {HISTORY_PAGE_SIZE} OFFSET {offset}")).map_err(sql_error)?;
        let mut clips = statement
            .query_map(rusqlite::params_from_iter(values.iter()), clip_from_row)
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        Self::fill_entries(&self.conn, &mut clips)?;
        Ok(HistoryPage {
            clips,
            history: HistorySummary {
                page,
                page_size: HISTORY_PAGE_SIZE,
                total,
                counts: global.counts,
                group_counts: global.group_counts,
                sources: global.sources,
                hosts: global.hosts,
                tags: global.tags,
            },
        })
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
        let mut clips = statement
            .query_map(rusqlite::params_from_iter(ids.iter()), clip_from_row)
            .map_err(sql_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?;
        Self::fill_entries(&self.conn, &mut clips)?;
        Ok(clips)
    }

    pub fn has_pending_ai_history(&self) -> Result<bool> {
        let revision = self.privacy_rules()?.revision;
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id WHERE c.kind NOT IN ('image','file','bundle') AND COALESCE(json_extract(p.metadata,'$.scanVersion'),0) != ?1)", [revision], |row| row.get(0)).map_err(sql_error)
    }

    pub fn clip(&self, id: i64) -> Result<Clip> {
        let clip = self
            .conn
            .query_row(&format!("{CLIP_SELECT} WHERE c.id=?1"), [id], clip_from_row)
            .optional()
            .map_err(sql_error)?
            .ok_or_else(|| "这条星笺已被删除。".to_string())?;
        let mut clips = vec![clip];
        Self::fill_entries(&self.conn, &mut clips)?;
        Ok(clips.remove(0))
    }
}
