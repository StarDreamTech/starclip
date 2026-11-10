use super::*;

pub(super) fn ensure_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS bundle_images (
        id INTEGER PRIMARY KEY,digest TEXT NOT NULL,width INTEGER NOT NULL,height INTEGER NOT NULL,
        bytes INTEGER NOT NULL,png BLOB NOT NULL,thumbnail BLOB NOT NULL);
        CREATE INDEX IF NOT EXISTS bundle_images_digest ON bundle_images(digest);
        CREATE TABLE IF NOT EXISTS clip_bundles (
        clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,digest TEXT NOT NULL,image_bytes INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS clip_bundles_digest ON clip_bundles(digest);
        CREATE TABLE IF NOT EXISTS clip_bundle_items (
        clip_id INTEGER NOT NULL REFERENCES clip_bundles(clip_id) ON DELETE CASCADE,
        position INTEGER NOT NULL,image_id INTEGER NOT NULL REFERENCES bundle_images(id),PRIMARY KEY(clip_id,position));
        CREATE INDEX IF NOT EXISTS clip_bundle_items_image ON clip_bundle_items(image_id);
        CREATE TRIGGER IF NOT EXISTS bundle_images_cleanup AFTER DELETE ON clip_bundle_items BEGIN
        DELETE FROM bundle_images WHERE id=old.image_id AND NOT EXISTS(SELECT 1 FROM clip_bundle_items WHERE image_id=old.image_id); END;")
        .map_err(sql_error)
}

fn put_asset(conn: &Connection, image: &PreparedImage) -> Result<i64> {
    let mut statement = conn.prepare("SELECT id,width,height,png FROM bundle_images WHERE digest=?1 AND length(png)<=?2 AND width=?3 AND height=?4 ORDER BY id").map_err(sql_error)?;
    let mut rows = statement
        .query(params![
            image.digest(),
            images::MAX_PNG_BYTES,
            image.metadata().width,
            image.metadata().height
        ])
        .map_err(sql_error)?;
    while let Some(row) = rows.next().map_err(sql_error)? {
        let png: Vec<u8> = row.get(3).map_err(sql_error)?;
        if image.matches_png(
            row.get(1).map_err(sql_error)?,
            row.get(2).map_err(sql_error)?,
            &png,
        )? {
            return row.get(0).map_err(sql_error);
        }
    }
    let m = image.metadata();
    conn.execute("INSERT INTO bundle_images(digest,width,height,bytes,png,thumbnail) VALUES(?1,?2,?3,?4,?5,?6)",params![image.digest(),m.width,m.height,m.bytes,image.png,image.thumbnail]).map_err(sql_error)?;
    Ok(conn.last_insert_rowid())
}

fn item_ids(conn: &Connection, id: i64) -> Result<Vec<i64>> {
    let mut statement = conn
        .prepare("SELECT image_id FROM clip_bundle_items WHERE clip_id=?1 ORDER BY position")
        .map_err(sql_error)?;
    let values = statement.query_map([id], |r| r.get(0)).map_err(sql_error)?;
    values.collect::<rusqlite::Result<_>>().map_err(sql_error)
}

impl Store {
    pub fn add_bundle(
        &self,
        text: &str,
        images: &[PreparedImage],
        title: &str,
        pinned: bool,
        private: bool,
        capture: Option<&CaptureContext>,
    ) -> Result<Clip> {
        self.insert_bundle(text, images, title, pinned, private, capture, None)
            .map(|(clip, _)| clip)
    }

    pub(super) fn insert_bundle(
        &self,
        text: &str,
        images: &[PreparedImage],
        title: &str,
        pinned: bool,
        private: bool,
        capture: Option<&CaptureContext>,
        imported: Option<&Clip>,
    ) -> Result<(Clip, bool)> {
        if images.is_empty() || images.len() > crate::image_transfer::MAX_IMAGES {
            return Err("图文组合需包含 1 至 50 张图片。".into());
        }
        validate_content(if text.is_empty() { "图集" } else { text }, title)?;
        let bytes = images.iter().try_fold(0usize, |sum, image| {
            sum.checked_add(image.png.len()).ok_or("图片大小超过限制。")
        })?;
        if bytes > crate::image_transfer::MAX_BYTES {
            return Err("所选图片总大小超过 100 MiB，请分批操作。".into());
        }
        let digest = format!(
            "bundle:{:x}",
            Sha256::digest(
                serde_json::to_vec(&(text, images.iter().map(|i| i.digest()).collect::<Vec<_>>()))
                    .map_err(|e| e.to_string())?
            )
        );
        let transaction = if self.conn.is_autocommit() {
            Some(self.conn.unchecked_transaction().map_err(sql_error)?)
        } else {
            None
        };
        let tx = &self.conn;
        let ids = images
            .iter()
            .map(|image| put_asset(&tx, image))
            .collect::<Result<Vec<_>>>()?;
        // The digest is only a bucket. Exact text and ordered asset identities decide equality.
        let mut existing = None;
        {
            let mut statement=tx.prepare("SELECT c.id,c.content FROM clip_bundles b JOIN clips c ON c.id=b.clip_id WHERE b.digest=?1 ORDER BY c.id").map_err(sql_error)?;
            let rows = statement
                .query_map([&digest], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(sql_error)?;
            for row in rows {
                let (id, content) = row.map_err(sql_error)?;
                if content == text && item_ids(&tx, id)? == ids {
                    existing = Some(id);
                    break;
                }
            }
        }
        let timestamp = now();
        let id = if let Some(id) = existing {
            tx.execute("UPDATE clips SET updated_at=CASE WHEN ?5 THEN updated_at ELSE ?2 END,pinned=MAX(pinned,?3),title=CASE WHEN title='' OR NOT ?5 THEN CASE WHEN ?4<>'' THEN ?4 ELSE title END ELSE title END WHERE id=?1",params![id,timestamp,pinned,title.trim(),imported.is_some()]).map_err(sql_error)?;
            id
        } else {
            tx.execute("INSERT INTO clips(content,title,kind,pinned,created_at,updated_at) VALUES(?1,?2,'bundle',?3,?4,?4)",params![text,title.trim(),pinned,timestamp]).map_err(sql_error)?;
            let id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO clip_bundles(clip_id,digest,image_bytes) VALUES(?1,?2,?3)",
                params![id, digest, bytes as i64],
            )
            .map_err(sql_error)?;
            for (position, image_id) in ids.iter().enumerate() {
                tx.execute(
                    "INSERT INTO clip_bundle_items(clip_id,position,image_id) VALUES(?1,?2,?3)",
                    params![id, position as i64, image_id],
                )
                .map_err(sql_error)?;
            }
            id
        };
        if let Some(context) = capture {
            Self::apply_capture(&tx, id, context)?;
            clip_stats::record(
                &tx,
                timestamp,
                "image",
                if existing.is_some() {
                    clip_stats::Event::Repeat
                } else {
                    clip_stats::Event::New
                },
                bytes as u64,
            )?;
        }
        if let Some(imported) = imported {
            tx.execute("UPDATE clips SET group_id=COALESCE(group_id,?2),keep_forever=MAX(keep_forever,?3) WHERE id=?1",params![id,imported.group_id,imported.keep_forever]).map_err(sql_error)?;
            if existing.is_none() {
                tx.execute(
                    "UPDATE clips SET created_at=?2,updated_at=?3,copy_count=?4 WHERE id=?1",
                    params![
                        id,
                        imported.created_at,
                        imported.updated_at,
                        imported.copy_count
                    ],
                )
                .map_err(sql_error)?;
            }
        }
        if private {
            let mut metadata = read_metadata(&tx, id)?;
            metadata.privacy_manual = true;
            put_metadata(&tx, id, &metadata)?;
        }
        scan_clip(&tx, id, &self.privacy_rules()?, false)?;
        if capture.is_some() {
            Self::prune_with_settings(&tx, &self.settings()?, timestamp)?;
        }
        let clip = self.clip(id)?;
        if let Some(tx) = transaction {
            tx.commit().map_err(sql_error)?;
        }
        Ok((clip, existing.is_none()))
    }

    pub fn bundle_from_clips(
        &self,
        ids: &[i64],
        text: &str,
        title: &str,
        allow_private: bool,
    ) -> Result<Clip> {
        crate::image_transfer::validate(self, ids, allow_private)?;
        let mut images = Vec::new();
        let mut private = false;
        let mut captions = Vec::new();
        let mut bytes = 0;
        for id in ids {
            let clip = self.clip(*id)?;
            private |= clip.is_private || clip.privacy_manual || !clip.privacy_reasons.is_empty();
            if clip.kind == "bundle"
                && !clip.content.is_empty()
                && !captions.contains(&clip.content)
            {
                captions.push(clip.content.clone());
            }
            for position in 0..self.image_count(*id)? {
                if images.len() >= crate::image_transfer::MAX_IMAGES {
                    return Err("图文组合需包含 1 至 50 张图片。".into());
                }
                let png = self.image_member_png(*id, position, false)?;
                bytes += png.len();
                if bytes > crate::image_transfer::MAX_BYTES {
                    return Err("所选图片总大小超过 100 MiB，请分批操作。".into());
                }
                images.push(images::prepare_png(&png)?);
            }
        }
        if !text.is_empty() && !captions.iter().any(|c| c == text) {
            captions.push(text.into());
        }
        self.add_bundle(&captions.join("\n\n"), &images, title, true, private, None)
    }

    pub fn image_count(&self, id: i64) -> Result<usize> {
        let clip = self.clip(id)?;
        match clip.kind.as_str() {
            "image" => Ok(1),
            "bundle" => Ok(clip.images.len()),
            _ => Err("这条记录不包含图片。".into()),
        }
    }

    pub(crate) fn bundle_matches(
        &self,
        id: i64,
        text: &str,
        images: &[PreparedImage],
    ) -> Result<bool> {
        let Ok(clip) = self.clip(id) else {
            return Ok(false);
        };
        if clip.kind != "bundle" || clip.content != text || clip.images.len() != images.len() {
            return Ok(false);
        }
        for (position, image) in images.iter().enumerate() {
            let png = self.image_member_png(id, position, false)?;
            if !image.matches_png(
                clip.images[position].width,
                clip.images[position].height,
                &png,
            )? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn update_bundle(
        &self,
        id: i64,
        text: &str,
        title: &str,
        group_id: Option<i64>,
        pinned: bool,
    ) -> Result<()> {
        validate_content(if text.is_empty() { "图集" } else { text }, title)?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        if self.clip(id)?.kind != "bundle" {
            return Err("这条记录不是图文组合。".into());
        }
        let digests = {
            let mut stmt=tx.prepare("SELECT i.digest FROM clip_bundle_items b JOIN bundle_images i ON i.id=b.image_id WHERE b.clip_id=?1 ORDER BY b.position").map_err(sql_error)?;
            let rows = stmt
                .query_map([id], |r| r.get::<_, String>(0))
                .map_err(sql_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?
        };
        let digest = format!(
            "bundle:{:x}",
            Sha256::digest(serde_json::to_vec(&(text, digests)).map_err(|e| e.to_string())?)
        );
        let members = item_ids(&tx, id)?;
        {
            let mut stmt=tx.prepare("SELECT c.id FROM clips c JOIN clip_bundles b ON b.clip_id=c.id WHERE c.id!=?1 AND c.content=?2 AND b.digest=?3").map_err(sql_error)?;
            let candidates = stmt
                .query_map(params![id, text, digest], |r| r.get::<_, i64>(0))
                .map_err(sql_error)?;
            for candidate in candidates {
                if item_ids(&tx, candidate.map_err(sql_error)?)? == members {
                    return Err("相同图文组合已存在，请编辑已有记录。".into());
                }
            }
        }
        tx.execute(
            "UPDATE clips SET content=?2,title=?3,group_id=?4,pinned=?5,updated_at=?6 WHERE id=?1",
            params![id, text, title.trim(), group_id, pinned, now()],
        )
        .map_err(sql_error)?;
        tx.execute(
            "UPDATE clip_bundles SET digest=?2 WHERE clip_id=?1",
            params![id, digest],
        )
        .map_err(sql_error)?;
        scan_clip(&tx, id, &self.privacy_rules()?, false)?;
        tx.commit().map_err(sql_error)
    }

    pub fn image_member_png(&self, id: i64, position: usize, thumbnail: bool) -> Result<Vec<u8>> {
        let kind: String = self
            .conn
            .query_row("SELECT kind FROM clips WHERE id=?1", [id], |r| r.get(0))
            .map_err(sql_error)?;
        if kind == "image" && position == 0 {
            return self.image_png(id, thumbnail);
        }
        if kind != "bundle" {
            return Err("图片位置无效。".into());
        }
        let column = if thumbnail { "thumbnail" } else { "png" };
        self.conn.query_row(&format!("SELECT i.{column} FROM bundle_images i JOIN clip_bundle_items b ON b.image_id=i.id WHERE b.clip_id=?1 AND b.position=?2 AND length(i.{column})<=?3"),params![id,position as i64,images::MAX_PNG_BYTES],|r|r.get(0)).optional().map_err(sql_error)?.ok_or("图片不存在或超过读取上限。".into())
    }

    pub(super) fn fill_bundle_images(conn: &Connection, clips: &mut [Clip]) -> Result<()> {
        for clip in clips.iter_mut().filter(|c| c.kind == "bundle") {
            let mut statement=conn.prepare("SELECT b.position,i.width,i.height,i.bytes FROM clip_bundle_items b JOIN bundle_images i ON i.id=b.image_id WHERE b.clip_id=?1 ORDER BY b.position").map_err(sql_error)?;
            let rows = statement
                .query_map([clip.id], |r| {
                    Ok((
                        r.get::<_, usize>(0)?,
                        ImageMetadata {
                            width: r.get(1)?,
                            height: r.get(2)?,
                            bytes: r.get(3)?,
                        },
                    ))
                })
                .map_err(sql_error)?;
            for row in rows {
                let (position, image) = row.map_err(sql_error)?;
                if position != clip.images.len() {
                    return Err("图文图片顺序损坏。".into());
                }
                clip.images.push(image);
            }
            if clip.images.is_empty() || clip.images.len() > crate::image_transfer::MAX_IMAGES {
                return Err("图文组合缺少图片或超过数量限制。".into());
            }
        }
        Ok(())
    }
}
