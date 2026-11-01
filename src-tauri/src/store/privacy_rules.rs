use super::*;

impl Store {
    pub fn privacy_rules(&self) -> Result<PrivacyRules> {
        read_privacy_rules(&self.conn)
    }

    pub fn save_privacy_rules(&self, requested: &PrivacyRules) -> Result<PrivacyRules> {
        let mut requested = requested.clone();
        for rule in &mut requested.rules {
            rule.tag = rule
                .tag
                .as_ref()
                .map(|tag| tag.trim().to_owned())
                .filter(|tag| !tag.is_empty());
        }
        requested.validate()?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let current = self.privacy_rules()?;
        if requested.revision != current.revision {
            return Err("规则已变化，请重新打开后再保存。".into());
        }
        let mut next = requested.clone();
        if current.rules == next.rules {
            return Ok(current);
        }
        next.revision = current
            .revision
            .checked_add(1)
            .ok_or("规则版本达到上限。")?;
        tx.execute("INSERT INTO preferences(key,value) VALUES('privacy_rules_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&next).map_err(|e| e.to_string())?]).map_err(sql_error)?;
        // Older scans immediately become ineligible for AI; only a completed new scan restores eligibility.
        tx.commit().map_err(sql_error)?;
        Ok(next)
    }

    pub fn set_clip_privacy(&self, id: i64, private: bool, reviewed: bool) -> Result<Clip> {
        if !private && !reviewed {
            return Err("解除隐私前需要在本机确认已经复核内容。".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        self.clip(id)?;
        let mut metadata = read_metadata(&tx, id)?;
        metadata.privacy_manual = private;
        put_metadata(&tx, id, &metadata)?;
        // A local review may remove stale reasons, never an active match.
        scan_clip(&tx, id, &self.privacy_rules()?, !private && reviewed)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    pub fn set_clip_tags(&self, id: i64, tags: &[String]) -> Result<Clip> {
        privacy::validate_tags(tags)?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        self.clip(id)?;
        let mut metadata = read_metadata(&tx, id)?;
        metadata.manual_tags = privacy::union(tags, &[]);
        put_metadata(&tx, id, &metadata)?;
        let clip = self.clip(id)?;
        tx.commit().map_err(sql_error)?;
        Ok(clip)
    }

    pub(super) fn privacy_rescan_plan(&self) -> Result<(PrivacyRescan, Vec<(i64, Metadata)>)> {
        let rules = self.privacy_rules()?;
        let mut statement = self
            .conn
            .prepare(&format!(
                "{CLIP_SELECT} ORDER BY c.updated_at DESC,c.id DESC"
            ))
            .map_err(sql_error)?;
        let clips = statement.query_map([], clip_from_row).map_err(sql_error)?;
        let mut digest = Sha256::new();
        digest.update(serde_json::to_vec(&rules).map_err(|e| e.to_string())?);
        let mut report = PrivacyRescan {
            token: String::new(),
            total: 0,
            private_count: 0,
            newly_private_count: 0,
            changed_count: 0,
        };
        let mut changes = Vec::new();
        for clip in clips {
            let clip = clip.map_err(sql_error)?;
            report.total += 1;
            let old = read_metadata(&self.conn, clip.id)?;
            digest.update(serde_json::to_vec(&clip).map_err(|e| e.to_string())?);
            digest.update(serde_json::to_vec(&old).map_err(|e| e.to_string())?);
            let mut next = old.scanned(&clip.content, &clip.title, &clip.kind, &rules, false);
            next.ensure_identity()?;
            report.private_count += usize::from(next.is_private());
            report.newly_private_count += usize::from(next.is_private() && !old.is_private());
            if old != next {
                report.changed_count += 1;
                changes.push((clip.id, next));
            }
        }
        report.token = format!("{:x}", digest.finalize());
        Ok((report, changes))
    }

    pub fn privacy_rescan_preview(&self) -> Result<PrivacyRescan> {
        self.privacy_rescan_plan().map(|(report, _)| report)
    }

    pub fn privacy_rescan_apply(&self, token: &str) -> Result<PrivacyRescan> {
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let (report, changes) = self.privacy_rescan_plan()?;
        if report.token != token {
            return Err("记录或规则已变化，请重新预览扫描结果。".into());
        }
        for (id, metadata) in changes {
            put_metadata(&tx, id, &metadata)?;
        }
        tx.commit().map_err(sql_error)?;
        Ok(report)
    }
}
