use super::*;

impl Store {
    /// Read-only validation must never initialize, migrate, or prune a selected database.
    pub fn validate_database(path: &Path) -> Result<()> {
        if !path.is_file() {
            return Err("所选目录缺少 starclip.sqlite3，未创建新数据库。".into());
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(sql_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(sql_error)?;
        if Self::check_schema_version(&conn)? == SCHEMA_VERSION {
            Self::validate_current_schema(&conn)?;
        }
        let tx = conn.unchecked_transaction().map_err(sql_error)?;
        let integrity: String = tx
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(sql_error)?;
        if integrity != "ok" {
            return Err("数据库完整性检查失败，未切换数据目录。".into());
        }
        // The original schema had no language column; normal startup upgrades it later.
        for query in [
            "SELECT id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count FROM clips LIMIT 0",
            "SELECT id,name,color FROM groups LIMIT 0",
            "SELECT key,value FROM preferences LIMIT 0",
        ] {
            tx.prepare(query).map_err(|_| "所选文件不是兼容的星笺数据库。".to_string())?;
        }
        if tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(sql_error)?
            .exists([])
            .map_err(sql_error)?
        {
            return Err("数据库分组引用损坏，未切换数据目录。".into());
        }
        let json: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        if let Some(json) = json {
            let settings: Settings =
                serde_json::from_str(&json).map_err(|_| "目标数据库的偏好设置损坏。")?;
            settings.validate()?;
        }
        Ok(())
    }

    /// SQLite includes committed WAL pages in this consistent standalone snapshot.
    pub fn snapshot(&self, target: &Path) -> Result<()> {
        let target = target.to_str().ok_or("目标路径不是有效的 Unicode 路径。")?;
        self.conn
            .execute("VACUUM main INTO ?1", [target])
            .map_err(sql_error)?;
        Ok(())
    }

    pub(super) fn validate_current_schema(conn: &Connection) -> Result<()> {
        for query in [
            "SELECT id,content,title,kind,pinned,group_id,created_at,updated_at,copy_count,language,keep_forever FROM clips LIMIT 0",
            "SELECT id,name,color FROM groups LIMIT 0",
            "SELECT key,value FROM preferences LIMIT 0",
            "SELECT clip_id,width,height,bytes,png,thumbnail FROM clip_images LIMIT 0",
            "SELECT clip_id,metadata FROM clip_privacy LIMIT 0",
        ] {
            conn.prepare(query).map_err(|_| "数据库结构损坏，请从恢复点恢复。".to_string())?;
        }
        Ok(())
    }

    pub(crate) fn check_schema_version(conn: &Connection) -> Result<u32> {
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(sql_error)?;
        if version > SCHEMA_VERSION {
            return Err(format!("此数据由较新版本的星笺创建（结构版本 {version}，当前支持 {SCHEMA_VERSION}）。请安装较新版本；原数据未更改。"));
        }
        Ok(version)
    }

    pub fn schema_version(&self) -> Result<u32> {
        Self::check_schema_version(&self.conn)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let directory = path.parent().ok_or("数据库路径缺少父目录。")?;
        Self::open_with_recovery(path, directory, &directory.join("recovery"))
    }

    pub(crate) fn open_with_recovery(
        path: &Path,
        selection: &Path,
        recovery: &Path,
    ) -> Result<Self> {
        let mut snapshot = None;
        if path.exists() {
            if fs::symlink_metadata(path)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("数据库不能是符号链接。".into());
            }
            // Inspect read-only before any writable connection or migration is opened.
            let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(sql_error)?;
            conn.busy_timeout(std::time::Duration::from_secs(3))
                .map_err(sql_error)?;
            let version = Self::check_schema_version(&conn)?;
            let existing: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clips')", [], |row| row.get(0)).map_err(sql_error)?;
            if existing && version < SCHEMA_VERSION {
                snapshot = Some(crate::storage::create_recovery_snapshot(
                    recovery,
                    path.parent().ok_or("数据库路径无效。")?,
                    selection,
                    version,
                    "before-migration",
                    |target| {
                        conn.execute(
                            "VACUUM main INTO ?1",
                            [target.to_str().ok_or("备份路径无效。")?],
                        )
                        .map_err(sql_error)?;
                        Ok(())
                    },
                )?);
            }
        }
        let result = Connection::open(path)
            .map_err(sql_error)
            .and_then(Self::from_connection);
        result.map_err(|error| match snapshot {
            Some(point) => format!("数据库升级失败：{error}\n升级前恢复点已保留：{}。可在启动恢复中选择该恢复点；不会覆盖原目录。", point.path),
            None => error,
        })
    }

    pub(super) fn from_connection(conn: Connection) -> Result<Self> {
        // SQLite lower() folds ASCII only; keep Unicode search consistent with the UI.
        conn.create_scalar_function(
            "starclip_contains",
            2,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            |context| {
                Ok(context
                    .get::<String>(0)?
                    .to_lowercase()
                    .contains(&context.get::<String>(1)?))
            },
        )
        .map_err(sql_error)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))
            .map_err(sql_error)?;
        let version = Self::check_schema_version(&conn)?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;")
            .map_err(sql_error)?;
        if version == SCHEMA_VERSION {
            Self::validate_current_schema(&conn)?;
            clip_stats::ensure_table(&conn)?;
            ensure_rich_table(&conn)?;
            crate::search_index::ensure(&conn)?;
            let store = Self { conn };
            store.prune_history()?;
            return Ok(store);
        }
        // Every schema change and its version marker commit together. An error rolls
        // back even early ADD COLUMN steps, preserving the pre-upgrade database.
        let tx = conn.unchecked_transaction().map_err(sql_error)?;
        let existing_database: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clips')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS groups (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, color TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS clips (
                id INTEGER PRIMARY KEY, content TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, keep_forever INTEGER NOT NULL DEFAULT 0,
                group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, copy_count INTEGER NOT NULL DEFAULT 0, language TEXT
            );
            CREATE INDEX IF NOT EXISTS clips_updated ON clips(updated_at DESC);
            CREATE TABLE IF NOT EXISTS preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);").map_err(sql_error)?;
        if !existing_database {
            tx.execute("INSERT INTO preferences(key,value) VALUES('onboarding_v1','false') ON CONFLICT(key) DO NOTHING", []).map_err(sql_error)?;
        }
        let columns = {
            let mut stmt = tx.prepare("PRAGMA table_info(clips)").map_err(sql_error)?;
            let columns = stmt
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(sql_error)?;
            columns
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?
        };
        if !columns.iter().any(|name| name == "language") {
            tx.execute("ALTER TABLE clips ADD COLUMN language TEXT", [])
                .map_err(sql_error)?;
        }
        if !columns.iter().any(|name| name == "keep_forever") {
            tx.execute(
                "ALTER TABLE clips ADD COLUMN keep_forever INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(sql_error)?;
        }
        // The original UNIQUE(content) mixed text and image identities. Rebuild once,
        // in one transaction, before any image table exists; preserve every original field.
        let legacy_unique: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_index_list('clips') WHERE origin='u')",
                [],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if legacy_unique {
            let has_images: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='clip_images')", [], |row| row.get(0)).map_err(sql_error)?;
            if has_images {
                return Err("图片数据库结构不兼容，原数据未更改。".into());
            }
            tx.execute_batch("CREATE TABLE clips_images_upgrade (
                id INTEGER PRIMARY KEY, content TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, keep_forever INTEGER NOT NULL DEFAULT 0,
                group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, copy_count INTEGER NOT NULL DEFAULT 0, language TEXT);
                INSERT INTO clips_images_upgrade(id,content,title,kind,pinned,keep_forever,group_id,created_at,updated_at,copy_count,language)
                    SELECT id,content,title,kind,pinned,keep_forever,group_id,created_at,updated_at,copy_count,language FROM clips;
                DROP TABLE clips;
                ALTER TABLE clips_images_upgrade RENAME TO clips;").map_err(sql_error)?;
        }
        tx.execute_batch("CREATE INDEX IF NOT EXISTS clips_updated ON clips(updated_at DESC);
            CREATE UNIQUE INDEX IF NOT EXISTS clips_text_content ON clips(content) WHERE kind<>'image';
            CREATE UNIQUE INDEX IF NOT EXISTS clips_image_content ON clips(content) WHERE kind='image';
            CREATE TABLE IF NOT EXISTS clip_images (
                clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,
                width INTEGER NOT NULL CHECK(width>0), height INTEGER NOT NULL CHECK(height>0),
                bytes INTEGER NOT NULL CHECK(bytes>0), png BLOB NOT NULL, thumbnail BLOB NOT NULL
            );").map_err(sql_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS clip_privacy (clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE, metadata TEXT NOT NULL);").map_err(sql_error)?;
        let rules = read_privacy_rules(&tx)?;
        let ids = {
            let mut stmt = tx.prepare("SELECT id FROM clips").map_err(sql_error)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(sql_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)?
        };
        for id in ids {
            scan_clip(&tx, id, &rules, false)?;
        }
        Self::validate_current_schema(&tx)?;
        if tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(sql_error)?
            .exists([])
            .map_err(sql_error)?
        {
            return Err("数据库引用损坏，升级已撤销。".into());
        }
        let json: Option<String> = tx
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let settings: Settings = match json {
            Some(json) => serde_json::from_str(&json).map_err(|_| "偏好设置损坏，升级已撤销。")?,
            None => Settings::default(),
        };
        settings.validate()?;
        Self::prune_with_settings(&tx, &settings, now())?;
        clip_stats::ensure_table(&tx)?;
        ensure_rich_table(&tx)?;
        crate::search_index::ensure(&tx)?;
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(Self { conn })
    }
}
