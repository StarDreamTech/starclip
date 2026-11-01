use super::*;

impl Store {
    pub fn add(&self, content: &str, title: &str, group_id: Option<i64>) -> Result<Clip> {
        self.add_protected(content, title, group_id, false)
    }

    pub(super) fn put_image(conn: &Connection, id: i64, image: &PreparedImage) -> Result<()> {
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
        self.insert_image(image, title, group_id, keep_forever, None)
    }

    /// Clipboard capture path: also counts the activity for 星迹统计 in the same transaction.
    pub fn capture_image(&self, image: &PreparedImage, context: &CaptureContext) -> Result<Clip> {
        self.insert_image(image, "", None, false, Some(context))
    }

    pub(super) fn insert_image(
        &self,
        image: &PreparedImage,
        title: &str,
        group_id: Option<i64>,
        keep_forever: bool,
        capture: Option<&CaptureContext>,
    ) -> Result<Clip> {
        validate_content(image.digest(), title)?;
        let timestamp = now();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let existed = capture.is_some() && Self::content_exists(&tx, image.digest(), true)?;
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
        if let Some(context) = capture {
            Self::apply_capture(&tx, id, context)?;
            let event = if existed {
                clip_stats::Event::Repeat
            } else {
                clip_stats::Event::New
            };
            clip_stats::record(&tx, timestamp, "image", event, image.metadata().bytes)?;
        }
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
        self.insert_text(content, title, group_id, keep_forever, None)
    }

    /// Clipboard capture path: also counts the activity for 星迹统计 in the same transaction.
    pub fn capture_text(&self, content: &str, context: &CaptureContext) -> Result<Clip> {
        self.capture_rich_text(content, None, context)
    }

    /// 文字采集；`html` 为同一次复制附带的格式，最新一次复制决定是否保留格式。
    pub fn capture_rich_text(
        &self,
        content: &str,
        html: Option<&str>,
        context: &CaptureContext,
    ) -> Result<Clip> {
        let html = html.filter(|html| !html.trim().is_empty() && html.len() <= MAX_RICH_BYTES);
        self.insert_text(
            content,
            "",
            None,
            false,
            Some((context, CaptureExtra::Html(html))),
        )
    }

    /// 文件采集：正文是换行连接的绝对路径，复制回去写入文件列表，不读取文件内容。
    pub fn capture_files(&self, paths: &[String], context: &CaptureContext) -> Result<Clip> {
        if paths.is_empty() || paths.len() > MAX_FILES {
            return Err(format!("一次复制的文件需为 1–{MAX_FILES} 个。"));
        }
        if paths
            .iter()
            .any(|path| path.trim().is_empty() || path.contains('\n'))
        {
            return Err("文件路径无效。".into());
        }
        self.insert_text(
            &paths.join("\n"),
            "",
            None,
            false,
            Some((context, CaptureExtra::Files)),
        )
    }

    pub fn clip_html(&self, id: i64) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT html FROM clip_rich WHERE clip_id=?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(sql_error)
    }

    /// 写入复制上下文；随后的 `scan_clip` 会带着来源域名重新匹配规则。
    pub(super) fn apply_capture(
        conn: &Connection,
        id: i64,
        context: &CaptureContext,
    ) -> Result<()> {
        if *context == CaptureContext::default() {
            return Ok(());
        }
        let mut metadata = read_metadata(conn, id)?;
        metadata.apply_capture(context);
        put_metadata(conn, id, &metadata)
    }

    pub(super) fn content_exists(conn: &Connection, content: &str, image: bool) -> Result<bool> {
        let predicate = if image {
            "kind='image'"
        } else {
            "kind<>'image'"
        };
        conn.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM clips WHERE content=?1 AND {predicate})"),
            [content],
            |row| row.get(0),
        )
        .map_err(sql_error)
    }

    pub(super) fn insert_text(
        &self,
        content: &str,
        title: &str,
        group_id: Option<i64>,
        keep_forever: bool,
        capture: Option<(&CaptureContext, CaptureExtra<'_>)>,
    ) -> Result<Clip> {
        validate_content(content, title)?;
        let kind = match capture {
            Some((_, CaptureExtra::Files)) => "file",
            _ => classify(content),
        };
        let timestamp = now();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let existed = capture.is_some() && Self::content_exists(&tx, content, false)?;
        tx.execute("INSERT INTO clips(content,title,kind,group_id,created_at,updated_at,keep_forever) VALUES(?1,?2,?3,?4,?5,?5,?6)
            ON CONFLICT(content) WHERE kind<>'image' DO UPDATE SET updated_at=excluded.updated_at,
            title=CASE WHEN excluded.title<>'' THEN excluded.title ELSE clips.title END,
            group_id=COALESCE(excluded.group_id,clips.group_id),
            keep_forever=MAX(clips.keep_forever,excluded.keep_forever)", params![content, title.trim(), kind, group_id, timestamp, keep_forever]).map_err(sql_error)?;
        let id = tx
            .query_row(
                "SELECT id FROM clips WHERE content=?1 AND kind<>'image'",
                [content],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if let Some((context, extra)) = capture {
            Self::apply_capture(&tx, id, context)?;
            match extra {
                CaptureExtra::Html(Some(html)) => {
                    tx.execute("INSERT INTO clip_rich(clip_id,html) VALUES(?1,?2) ON CONFLICT(clip_id) DO UPDATE SET html=excluded.html", params![id, html]).map_err(sql_error)?;
                }
                CaptureExtra::Html(None) => {
                    tx.execute("DELETE FROM clip_rich WHERE clip_id=?1", [id])
                        .map_err(sql_error)?;
                }
                CaptureExtra::Files => {}
            }
            // A repeat keeps the stored kind, e.g. text later saved as code.
            let kind: String = tx
                .query_row("SELECT kind FROM clips WHERE id=?1", [id], |row| row.get(0))
                .map_err(sql_error)?;
            let event = if existed {
                clip_stats::Event::Repeat
            } else {
                clip_stats::Event::New
            };
            clip_stats::record(&tx, timestamp, &kind, event, content.chars().count() as u64)?;
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
                match self.clip(id)?.kind.as_str() {
                    "image" => return Err("图片不能转换为代码，请新建代码片段。".into()),
                    "file" => return Err("文件记录不能转换为代码，请新建代码片段。".into()),
                    _ => {}
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
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let kind: Option<String> = tx
            .query_row("SELECT kind FROM clips WHERE id=?1", [id], |row| row.get(0))
            .optional()
            .map_err(sql_error)?;
        if let Some(kind) = kind {
            tx.execute("UPDATE clips SET copy_count=copy_count+1 WHERE id=?1", [id])
                .map_err(sql_error)?;
            clip_stats::record(&tx, now(), &kind, clip_stats::Event::Reuse, 0)?;
        }
        tx.commit().map_err(sql_error)
    }

    pub fn clip_stats(&self, days: u32, utc_offset_minutes: i32) -> Result<ClipStats> {
        clip_stats::activity(
            &self.conn,
            now(),
            days,
            utc_offset_minutes,
            PRIVATE_PREDICATE,
        )
    }

    pub fn clear_clip_stats(&self) -> Result<()> {
        clip_stats::clear(&self.conn)
    }
}
