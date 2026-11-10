use super::*;

impl Store {
    pub fn export(&self) -> Result<String> {
        // Bound allocations before loading histories or image BLOBs. Serialized JSON is
        // checked again because base64 and metadata make it larger than the stored content.
        let (count, bytes): (usize, u64) = self.conn.query_row(
            "SELECT count(*),COALESCE(sum(CASE WHEN c.kind='image' THEN COALESCE(i.bytes,0) WHEN c.kind='bundle' THEN COALESCE((SELECT image_bytes FROM clip_bundles WHERE clip_id=c.id),0)+length(CAST(c.content AS BLOB)) ELSE length(CAST(c.content AS BLOB)) END + length(CAST(c.title AS BLOB))),0) FROM clips c LEFT JOIN clip_images i ON i.clip_id=c.id",
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
        let mut clips = self.clips()?;
        // Backup ranks describe relative order, independent of row IDs assigned on import.
        {
            let mut favorites: Vec<_> = clips.iter_mut().filter(|clip| clip.pinned).collect();
            favorites.sort_by(|a, b| {
                a.favorite_position
                    .cmp(&b.favorite_position)
                    .then_with(|| b.id.cmp(&a.id))
            });
            for (index, clip) in favorites.into_iter().enumerate() {
                clip.favorite_position = Some((index as i64 + 1) * 1024);
            }
        }
        check_backup_image_limits(&clips)?;
        let groups = self.groups()?;
        let mut images = Vec::new();
        let mut bundles = Vec::new();
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
            if clip.kind == "bundle" {
                let mut pngs = Vec::new();
                for (position, metadata) in clip.images.iter().enumerate() {
                    let png = self.image_member_png(clip.id, position, false)?;
                    let prepared = images::prepare_png(&png)?;
                    if metadata.width != prepared.metadata().width
                        || metadata.height != prepared.metadata().height
                        || metadata.bytes != png.len() as u64
                    {
                        return Err("图文图片数据与尺寸不符。".into());
                    }
                    let encoded = STANDARD.encode(png);
                    encoded_bytes = encoded_bytes.saturating_add(encoded.len());
                    check_backup_limits(encoded_bytes, 0, 0)?;
                    pngs.push(encoded);
                }
                bundles.push(BackupBundle {
                    clip_id: clip.id,
                    images: pngs,
                });
            }
        }
        let json = serde_json::to_string_pretty(&Backup {
            format: "starclip".into(),
            version: 5,
            exported_at: now(),
            clips,
            groups,
            images,
            bundles,
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
            || ![1, 2, 3, 4, 5].contains(&backup.version)
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
            if clip.favorite_position.is_some_and(|position| {
                !(1..=super::favorites::MAX_FAVORITE_POSITION).contains(&position)
            }) {
                return Err("备份中的收藏排序无效。".into());
            }
            validate_content(
                if clip.kind == "bundle" && clip.content.is_empty() {
                    "图集"
                } else {
                    &clip.content
                },
                &clip.title,
            )?;
            if clip.kind == "bundle"
                && (backup.version < 5
                    || clip.images.is_empty()
                    || clip.images.len() > crate::image_transfer::MAX_IMAGES
                    || clip.image.is_some()
                    || clip.language.is_some())
            {
                return Err("备份图文组合格式无效。".into());
            }
            if clip.kind != "bundle" && !clip.images.is_empty() {
                return Err("备份非图文记录带有多图数据。".into());
            }
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
            if clip.kind == "file" {
                let content = match clip.entries.as_deref() {
                    Some(entries) => file_content(entries)?,
                    None if backup.version < 4 => file_content(&unknown_entries(&clip.content)?)?,
                    None => return Err("备份文件记录缺少路径列表。".into()),
                };
                if content != clip.content || clip.language.is_some() {
                    return Err("备份文件记录与路径列表不一致。".into());
                }
            } else if clip.entries.is_some() {
                return Err("备份中非文件记录带有路径列表。".into());
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
        let mut prepared_bundles = HashMap::new();
        for bundle in backup.bundles {
            let clip = backup
                .clips
                .iter()
                .find(|c| c.id == bundle.clip_id && c.kind == "bundle")
                .ok_or("备份图文图片没有对应记录。")?;
            if prepared_bundles.contains_key(&bundle.clip_id)
                || bundle.images.len() != clip.images.len()
            {
                return Err("备份图文图片重复或数量不符。".into());
            }
            let mut list = Vec::new();
            let mut bytes = 0;
            for (encoded, metadata) in bundle.images.into_iter().zip(&clip.images) {
                let png = STANDARD
                    .decode(encoded)
                    .map_err(|_| "备份图文图片编码无效。")?;
                let image = images::prepare_png(&png)?;
                if (metadata.width, metadata.height, metadata.bytes)
                    != (
                        image.metadata().width,
                        image.metadata().height,
                        png.len() as u64,
                    )
                {
                    return Err("备份图文图片尺寸不符。".into());
                }
                bytes += image.png.len();
                if bytes > crate::image_transfer::MAX_BYTES {
                    return Err("所选图片总大小超过 100 MiB，请分批操作。".into());
                }
                list.push(image);
            }
            prepared_bundles.insert(bundle.clip_id, list);
        }
        if backup
            .clips
            .iter()
            .any(|c| c.kind == "bundle" && !prepared_bundles.contains_key(&c.id))
        {
            return Err("备份缺少图文原图。".into());
        }
        // All image bytes are validated before the transaction creates any group or clip.
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
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
            let predicate = identity_predicate(&clip.kind);
            let existed: Option<i64> = if clip.kind == "bundle" {
                None
            } else if is_image {
                Self::find_image(&tx, prepared.get(&clip.id).ok_or("备份缺少原图。")?)?
            } else {
                tx.query_row(
                    &format!("SELECT id FROM clips WHERE content=?1 AND {predicate}"),
                    [&clip.content],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sql_error)?
            };
            let kind = if is_image {
                "image"
            } else if clip.language.is_some() {
                "code"
            } else if clip.kind == "file" && clip.content.lines().count() <= MAX_FILES {
                "file"
            } else {
                classify(&clip.content)
            };
            let mut new_record = existed.is_none();
            let id = if clip.kind == "bundle" {
                let mut imported_clip = clip.clone();
                imported_clip.group_id = group_id;
                let (saved, is_new) = self.insert_bundle(
                    &clip.content,
                    prepared_bundles.get(&clip.id).ok_or("备份缺少图文原图。")?,
                    &clip.title,
                    clip.pinned,
                    false,
                    None,
                    Some(&imported_clip),
                )?;
                if is_new {
                    imported += 1;
                }
                new_record = is_new;
                saved.id
            } else if is_image {
                if let Some(id) = existed {
                    tx.execute("UPDATE clips SET pinned=MAX(pinned,?2),keep_forever=MAX(keep_forever,?3),group_id=COALESCE(group_id,?4),title=CASE WHEN title='' THEN ?5 ELSE title END WHERE id=?1",params![id,clip.pinned,clip.keep_forever,group_id,clip.title]).map_err(sql_error)?;
                    id
                } else {
                    tx.execute("INSERT INTO clips(content,title,kind,pinned,group_id,created_at,updated_at,copy_count,keep_forever) VALUES(?1,?2,'image',?3,?4,?5,?6,?7,?8)",params![clip.content,clip.title,clip.pinned,group_id,clip.created_at,clip.updated_at,clip.copy_count,clip.keep_forever]).map_err(sql_error)?;
                    tx.last_insert_rowid()
                }
            } else {
                tx.execute(&format!("INSERT INTO clips(content,title,kind,pinned,group_id,created_at,updated_at,copy_count,language,keep_forever) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                ON CONFLICT(content) WHERE {predicate} DO UPDATE SET pinned=MAX(clips.pinned,excluded.pinned),
                keep_forever=MAX(clips.keep_forever,excluded.keep_forever),
                group_id=COALESCE(clips.group_id,excluded.group_id),
                title=CASE WHEN clips.title='' THEN excluded.title ELSE clips.title END,
                language=COALESCE(clips.language,excluded.language),
                kind=CASE WHEN clips.language IS NOT NULL OR excluded.language IS NOT NULL THEN 'code' ELSE clips.kind END"),
                params![clip.content, clip.title, kind, clip.pinned, group_id, clip.created_at, clip.updated_at, clip.copy_count, clip.language, clip.keep_forever]).map_err(sql_error)?;
                existed.unwrap_or_else(|| tx.last_insert_rowid())
            };
            if clip.pinned {
                tx.execute("UPDATE clip_favorites SET on_top=MAX(on_top,?2),position=CASE WHEN ?4 THEN COALESCE(?3,position) ELSE position END WHERE clip_id=?1", params![id,clip.on_top,clip.favorite_position,new_record]).map_err(sql_error)?;
            }
            if let Some(image) = prepared.remove(&clip.id) {
                Self::put_image(&tx, id, &image)?;
            }
            if clip.kind == "file" {
                let mut entries = match clip.entries.as_ref() {
                    Some(entries) => entries.clone(),
                    None => unknown_entries(&clip.content)?,
                };
                if existed.is_some() {
                    // A backup can fill unknown kinds, but it must not erase or
                    // reinterpret known classifications already stored for this path.
                    let known = {
                        let mut statement = tx.prepare("SELECT path,entry_kind FROM clip_files WHERE clip_id=?1 AND entry_kind IN ('file','directory')").map_err(sql_error)?;
                        let rows = statement
                            .query_map([id], |row| {
                                let kind: String = row.get(1)?;
                                Ok((
                                    row.get::<_, String>(0)?,
                                    if kind == "file" {
                                        FileEntryKind::File
                                    } else {
                                        FileEntryKind::Directory
                                    },
                                ))
                            })
                            .map_err(sql_error)?;
                        rows.collect::<rusqlite::Result<HashMap<_, _>>>()
                            .map_err(sql_error)?
                    };
                    for entry in &mut entries {
                        if let Some(kind) = known.get(&entry.path) {
                            entry.entry_kind = *kind;
                        }
                    }
                }
                Self::put_file_entries(&tx, id, &entries)?;
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
            if existed.is_none() && clip.kind != "bundle" {
                imported += 1;
            }
        }
        // Import keeps the entire backup; capture, startup and maintenance apply retention later.
        tx.commit().map_err(sql_error)?;
        Ok(imported)
    }
}
