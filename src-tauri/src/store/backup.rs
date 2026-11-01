use super::*;

impl Store {
    pub fn export(&self) -> Result<String> {
        // Bound allocations before loading histories or image BLOBs. Serialized JSON is
        // checked again because base64 and metadata make it larger than the stored content.
        let (count, bytes): (usize, u64) = self.conn.query_row(
            "SELECT count(*),COALESCE(sum(CASE WHEN c.kind='image' THEN COALESCE(i.bytes,0) ELSE length(CAST(c.content AS BLOB)) END + length(CAST(c.title AS BLOB))),0) FROM clips c LEFT JOIN clip_images i ON i.clip_id=c.id",
            [], |row| Ok((row.get(0)?, row.get(1)?))).map_err(sql_error)?;
        let group_count: usize = self
            .conn
            .query_row("SELECT count(*) FROM groups", [], |row| row.get(0))
            .map_err(sql_error)?;
        check_backup_limits(
            usize::try_from(bytes).unwrap_or(usize::MAX),
            count,
            group_count,
        )?;
        let clips = self.clips()?;
        check_backup_image_limits(&clips)?;
        let groups = self.groups()?;
        let mut images = Vec::new();
        let mut encoded_bytes = 0usize;
        for clip in &clips {
            if clip.kind == "image" {
                let png = self.image_png(clip.id, false)?;
                let metadata = clip
                    .image
                    .as_ref()
                    .ok_or("图片元数据缺失，无法导出完整备份。")?;
                let (width, height, rgba) = images::decode_png(&png)?;
                if metadata.width != width
                    || metadata.height != height
                    || metadata.bytes != png.len() as u64
                    || images::fingerprint_rgba(width, height, &rgba)? != clip.content
                {
                    return Err("图片数据与元数据不符，未导出损坏的备份。".into());
                }
                let png = STANDARD.encode(png);
                encoded_bytes = encoded_bytes.saturating_add(png.len());
                check_backup_limits(encoded_bytes, 0, 0)?;
                images.push(BackupImage {
                    clip_id: clip.id,
                    png,
                });
            }
        }
        let json = serde_json::to_string_pretty(&Backup {
            format: "starclip".into(),
            version: 3,
            exported_at: now(),
            clips,
            groups,
            images,
            privacy_rules: Some(self.privacy_rules()?),
        })
        .map_err(|e| e.to_string())?;
        check_backup_limits(json.len(), 0, 0)?;
        Ok(json)
    }

    pub fn import(&mut self, json: &str) -> Result<usize> {
        check_backup_limits(json.len(), 0, 0)?;
        let backup: Backup = serde_json::from_str(json)
            .map_err(|_| "无法读取备份文件，请选择星笺导出的 JSON 文件。")?;
        if backup.format != "starclip"
            || ![1, 2, 3].contains(&backup.version)
            || (backup.version == 1 && !backup.images.is_empty())
        {
            return Err("不支持的备份版本。".into());
        }
        check_backup_limits(json.len(), backup.clips.len(), backup.groups.len())?;
        if let Some(rules) = &backup.privacy_rules {
            rules.validate()?;
        }
        let current_rules = self.privacy_rules()?;
        let mut active_rules = current_rules.clone();
        if let Some(imported_rules) = &backup.privacy_rules {
            for (index, imported) in imported_rules.rules.iter().enumerate() {
                // Re-importing the same policy does not accumulate duplicate drafts.
                if active_rules.rules.iter().any(|local| {
                    local.name == imported.name
                        && local.condition == imported.condition
                        && local.value == imported.value
                        && local.tag == imported.tag
                        && local.mark_private == imported.mark_private
                }) {
                    continue;
                }
                let mut draft = imported.clone();
                draft.enabled = false;
                let mut suffix = index;
                loop {
                    draft.id = format!("import-{}-{suffix}", current_rules.revision);
                    if !active_rules.rules.iter().any(|rule| rule.id == draft.id) {
                        break;
                    }
                    suffix += 1;
                }
                active_rules.rules.push(draft);
            }
            if active_rules.rules != current_rules.rules {
                active_rules.revision = current_rules
                    .revision
                    .checked_add(1)
                    .ok_or("规则版本达到上限。")?;
            }
            active_rules.validate()?;
        }
        let mut group_ids = HashSet::new();
        for group in &backup.groups {
            validate_group(&group.name, &group.color)?;
            if !group_ids.insert(group.id) {
                return Err("备份存在重复分组编号。".into());
            }
        }
        let mut clip_ids = HashSet::new();
        for clip in &backup.clips {
            validate_content(&clip.content, &clip.title)?;
            validate_backup_tags(&clip.tags)?;
            validate_backup_tags(&clip.manual_tags)?;
            if clip.privacy_reasons.len() > 128
                || clip.privacy_reasons.iter().any(|reason| reason.len() > 512)
            {
                return Err("备份隐私原因无效。".into());
            }
            if !clip_ids.insert(clip.id) {
                return Err("备份存在重复星笺编号。".into());
            }
            if let Some(language) = &clip.language {
                validate_language(language)?;
            }
            if clip.group_id.is_some_and(|id| !group_ids.contains(&id))
                || clip.created_at < 0
                || clip.updated_at < 0
                || clip.copy_count < 0
            {
                return Err("备份包含无效分组或记录。".into());
            }
            if (clip.kind == "image"
                && (backup.version < 2 || clip.image.is_none() || clip.language.is_some()))
                || (clip.kind != "image" && clip.image.is_some())
            {
                return Err("备份图片类型或元数据无效。".into());
            }
        }
        let mut prepared = HashMap::new();
        let mut prepared_bytes = 0usize;
        check_backup_image_limits(&backup.clips)?;
        let image_clips: HashMap<_, _> = backup
            .clips
            .iter()
            .filter(|clip| clip.kind == "image")
            .map(|clip| (clip.id, clip))
            .collect();
        for image in backup.images {
            let clip = image_clips
                .get(&image.clip_id)
                .ok_or("备份图片没有对应的图片记录。")?;
            if prepared.contains_key(&image.clip_id) {
                return Err("备份存在重复图片。".into());
            }
            let png = STANDARD
                .decode(&image.png)
                .map_err(|_| "备份图片的 Base64 数据损坏。")?;
            let value = images::prepare_png(&png)?;
            let metadata = clip.image.as_ref().unwrap();
            if metadata.width != value.metadata().width
                || metadata.height != value.metadata().height
                || metadata.bytes != png.len() as u64
                || clip.content != value.digest()
            {
                return Err("备份图片的像素、尺寸或摘要不匹配。".into());
            }
            prepared_bytes = prepared_bytes.saturating_add(value.png.len());
            check_backup_limits(prepared_bytes, 0, 0)?;
            prepared.insert(image.clip_id, value);
        }
        if backup
            .clips
            .iter()
            .any(|clip| clip.kind == "image" && !prepared.contains_key(&clip.id))
        {
            return Err("备份缺少原始图片，未导入不完整记录。".into());
        }
        // All image bytes are validated before the transaction creates any group or clip.
        let tx = self.conn.transaction().map_err(sql_error)?;
        if active_rules != current_rules {
            tx.execute("INSERT INTO preferences(key,value) VALUES('privacy_rules_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[serde_json::to_string(&active_rules).map_err(|e|e.to_string())?]).map_err(sql_error)?;
        }
        let mut group_mapping = HashMap::new();
        for group in backup.groups {
            tx.execute(
                "INSERT INTO groups(name,color) VALUES(?1,?2) ON CONFLICT(name) DO NOTHING",
                params![group.name.trim(), group.color],
            )
            .map_err(sql_error)?;
            let id: i64 = tx
                .query_row(
                    "SELECT id FROM groups WHERE name=?1",
                    [group.name.trim()],
                    |row| row.get(0),
                )
                .map_err(sql_error)?;
            group_mapping.insert(group.id, id);
        }
        let mut imported = 0;
        for clip in backup.clips {
            let group_id = clip.group_id.and_then(|id| group_mapping.get(&id).copied());
            let is_image = clip.kind == "image";
            let predicate = if is_image {
                "kind='image'"
            } else {
                "kind<>'image'"
            };
            let existed: Option<i64> = tx
                .query_row(
                    &format!("SELECT id FROM clips WHERE content=?1 AND {predicate}"),
                    [&clip.content],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sql_error)?;
            let kind = if is_image {
                "image"
            } else if clip.language.is_some() {
                "code"
            } else if clip.kind == "file" && clip.content.lines().count() <= MAX_FILES {
                "file"
            } else {
                classify(&clip.content)
            };
            tx.execute(&format!("INSERT INTO clips(content,title,kind,pinned,group_id,created_at,updated_at,copy_count,language,keep_forever) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                ON CONFLICT(content) WHERE {predicate} DO UPDATE SET pinned=MAX(clips.pinned,excluded.pinned),
                keep_forever=MAX(clips.keep_forever,excluded.keep_forever),
                group_id=COALESCE(clips.group_id,excluded.group_id),
                title=CASE WHEN clips.title='' THEN excluded.title ELSE clips.title END,
                language=COALESCE(clips.language,excluded.language),
                kind=CASE WHEN clips.language IS NOT NULL OR excluded.language IS NOT NULL THEN 'code' ELSE clips.kind END"),
                params![clip.content, clip.title, kind, clip.pinned, group_id, clip.created_at, clip.updated_at, clip.copy_count, clip.language, clip.keep_forever]).map_err(sql_error)?;
            let id = existed.unwrap_or_else(|| tx.last_insert_rowid());
            if let Some(image) = prepared.remove(&clip.id) {
                Self::put_image(&tx, id, &image)?;
            }
            let mut metadata = read_metadata(&tx, id)?;
            metadata.privacy_manual |=
                clip.privacy_manual || (clip.is_private && clip.privacy_reasons.is_empty());
            metadata.privacy_reasons =
                privacy::union(&metadata.privacy_reasons, &clip.privacy_reasons);
            let manual_tags = if backup.version >= 3 {
                &clip.manual_tags
            } else {
                &clip.tags
            };
            metadata.manual_tags = privacy::union(&metadata.manual_tags, manual_tags);
            for app in clip.app_tags.iter().take(privacy::MAX_APP_TAGS) {
                metadata.apply_capture(&CaptureContext {
                    app: Some(app.clone()),
                    ..CaptureContext::default()
                });
            }
            if metadata.origin_url.is_empty() {
                metadata.apply_capture(&CaptureContext {
                    origin_url: clip.origin_url.clone(),
                    ..CaptureContext::default()
                });
            }
            // Imported scan version/classification cannot grant trust. Reclassify the final merged row.
            put_metadata(&tx, id, &metadata)?;
            scan_clip(&tx, id, &active_rules, false)?;
            if existed.is_none() {
                imported += 1;
            }
        }
        // Import keeps the entire backup; capture, startup and maintenance apply retention later.
        tx.commit().map_err(sql_error)?;
        Ok(imported)
    }
}
