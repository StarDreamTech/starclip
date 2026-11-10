//! Retention applies only to recognized automatic recovery archives.
use super::*;

const KEEP_AUTOMATIC: usize = 3;
const PROTECT_AGE_MS: u64 = 7 * 24 * 60 * 60 * 1000;

impl Storage {
    /// Called only after successful startup, under the existing storage write gate.
    pub fn cleanup_automatic_recovery_points(&mut self, now: u64) -> Result<usize> {
        self.writable()?;
        let result = self.prune_automatic_recovery_points(now);
        self.recovery_cleanup_error = result
            .as_ref()
            .err()
            .map(|_| "旧自动恢复点清理未完成，将在下次维护时重试；当前数据未更改。".into());
        result
    }

    fn prune_automatic_recovery_points(&self, now: u64) -> Result<usize> {
        let root = self.recovery_directory();
        if !root.exists() {
            return Ok(0);
        }
        let metadata = fs::symlink_metadata(&root).map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("恢复目录无效。".into());
        }
        let current = self.current.canonicalize().map_err(io_error)?;
        let current_version = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|_| "当前应用版本无效。".to_string())?;
        let mut points = Vec::new();
        for entry in fs::read_dir(&root).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok((path, manifest)) = read_manifest(&root, &id) else {
                continue;
            };
            let Ok(app_version) = semver::Version::parse(&manifest.app_version) else {
                continue;
            };
            if manifest.format_version != 1
                || manifest.schema_version > SCHEMA_VERSION
                || app_version > current_version
                || manifest.created_at == 0
                || manifest.created_at > now
                || !matches!(
                    manifest.reason.as_str(),
                    "before-update" | "before-migration" | "before-restore"
                )
                || current.starts_with(path.canonicalize().map_err(io_error)?)
            {
                continue;
            }
            // Unknown/extra files, links or size mismatches are preserved for manual review.
            if !matches_manifest_sizes(&path, &manifest) {
                continue;
            }
            let upgraded =
                manifest.schema_version < SCHEMA_VERSION || app_version < current_version;
            points.push((path, manifest, upgraded));
        }
        points.sort_by(|a, b| {
            // Count protection applies to current-version points independently of removed versions.
            a.2.cmp(&b.2)
                .then_with(|| b.1.created_at.cmp(&a.1.created_at))
                .then_with(|| b.1.id.cmp(&a.1.id))
        });
        let candidates: Vec<_> = points
            .into_iter()
            .enumerate()
            .filter(|(index, (_, manifest, upgraded))| {
                *upgraded
                    || (*index >= KEEP_AUTOMATIC && now - manifest.created_at >= PROTECT_AGE_MS)
            })
            .map(|(_, point)| point)
            .collect();
        if candidates.is_empty() {
            return Ok(0);
        }
        // The new working database must be usable before discarding any rollback copy.
        let database = current.join(DATABASE);
        Store::validate_database(&database)?;
        let conn = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|error| error.to_string())?;
        if Store::check_schema_version(&conn)? != SCHEMA_VERSION {
            return Err("当前数据库尚未完成升级，自动恢复点已保留。".into());
        }
        let mut removed = 0;
        for (path, manifest, _) in candidates {
            // Recheck ownership just before removal, including the manual/automatic reason.
            let Ok((checked, current_manifest)) = read_manifest(&root, &manifest.id) else {
                continue;
            };
            if current_manifest != manifest
                || checked != path
                || current.starts_with(checked.canonicalize().map_err(io_error)?)
                || !matches_manifest_sizes(&checked, &manifest)
            {
                continue;
            }
            fs::remove_dir_all(&path).map_err(io_error)?;
            removed += 1;
        }
        Ok(removed)
    }
}

fn matches_manifest_sizes(path: &Path, manifest: &RecoveryManifest) -> bool {
    let Ok(files) = recovery_inventory(path, false) else {
        return false;
    };
    files.len() == manifest.files.len()
        && files.iter().all(|(name, file)| {
            manifest
                .files
                .get(name)
                .is_some_and(|known| known.bytes == file.bytes)
        })
        && files
            .values()
            .try_fold(0u64, |sum, file| sum.checked_add(file.bytes))
            == Some(manifest.total_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::Scratch;

    fn point(storage: &Storage, store: &Store, reason: &str, created_at: u64) -> RecoveryPoint {
        let point = storage.create_recovery_point(store, reason).unwrap();
        let (path, mut manifest) = read_manifest(&storage.recovery_directory(), &point.id).unwrap();
        manifest.created_at = created_at;
        fs::write(
            path.join("snapshot.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        point
    }

    fn version(point: &RecoveryPoint, schema: u32, app: &str) {
        let path = Path::new(&point.path).join("snapshot.json");
        let mut manifest: RecoveryManifest =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest.schema_version = schema;
        manifest.app_version = app.into();
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn recovery_cleanup_removes_previous_versions_without_count_or_age_protection() {
        let scratch = Scratch::new();
        let mut storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let store = storage.open_store().unwrap();
        let clip = store
            .add("Original current data after upgrade", "", None)
            .unwrap();
        let image = crate::images::prepare_rgba(1, 1, &[31, 72, 128, 255]).unwrap();
        let picture = store
            .add_image(&image, "Original current image", None, true)
            .unwrap();
        create_private_directory(&storage.current.join("fonts")).unwrap();
        fs::write(
            storage.current.join("fonts/original.fixture"),
            b"Original font bytes",
        )
        .unwrap();
        let now = 20 * PROTECT_AGE_MS;
        let current: Vec<_> = (1..=3)
            .map(|time| point(&storage, &store, "before-restore", time))
            .collect();
        let old_app = point(&storage, &store, "before-update", now - 1);
        version(&old_app, SCHEMA_VERSION, "0.0.1");
        let old_schema = point(&storage, &store, "before-migration", now - 2);
        version(&old_schema, SCHEMA_VERSION - 1, env!("CARGO_PKG_VERSION"));
        let old_restore = point(&storage, &store, "before-restore", now - 3);
        version(&old_restore, SCHEMA_VERSION - 1, "0.0.1");
        let manual = point(&storage, &store, "manual", now - 4);
        version(&manual, SCHEMA_VERSION - 1, "0.0.1");
        let future = point(&storage, &store, "before-update", now - 5);
        version(&future, SCHEMA_VERSION - 1, "999.0.0");
        let unknown = point(&storage, &store, "before-update", now - 6);
        version(&unknown, SCHEMA_VERSION - 1, "unknown");
        let before = fs::read(storage.current.join(DATABASE)).unwrap();
        assert_eq!(storage.cleanup_automatic_recovery_points(now).unwrap(), 3);
        for point in [&old_app, &old_schema, &old_restore] {
            assert!(!Path::new(&point.path).exists());
        }
        for point in [&manual, &future, &unknown] {
            assert!(Path::new(&point.path).exists());
        }
        assert!(current.iter().all(|point| Path::new(&point.path).exists()));
        assert_eq!(fs::read(storage.current.join(DATABASE)).unwrap(), before);
        assert_eq!(store.clip(clip.id).unwrap().content, clip.content);
        assert_eq!(store.image_png(picture.id, false).unwrap(), image.png);
        assert_eq!(
            fs::read(storage.current.join("fonts/original.fixture")).unwrap(),
            b"Original font bytes"
        );
        // A recent same-version installation attempt remains available for retry.
        let pending = point(&storage, &store, "before-update", now);
        assert_eq!(storage.cleanup_automatic_recovery_points(now).unwrap(), 1);
        assert!(Path::new(&pending.path).exists());
    }

    #[test]
    fn recovery_cleanup_keeps_upgrade_points_until_current_database_passes_validation() {
        let scratch = Scratch::new();
        let mut storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let store = storage.open_store().unwrap();
        store.add("Original validation fixture", "", None).unwrap();
        let point = point(&storage, &store, "before-update", 1);
        version(&point, SCHEMA_VERSION, "0.0.1");
        let conn = rusqlite::Connection::open(storage.current.join(DATABASE)).unwrap();
        conn.execute(
            "INSERT INTO preferences(key,value) VALUES('settings','invalid original fixture')",
            [],
        )
        .unwrap();
        assert!(storage.cleanup_automatic_recovery_points(2).is_err());
        assert!(Path::new(&point.path).exists());
        conn.execute("DELETE FROM preferences WHERE key='settings'", [])
            .unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        assert!(storage.cleanup_automatic_recovery_points(2).is_err());
        assert!(Path::new(&point.path).exists());
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .unwrap();
        assert_eq!(storage.cleanup_automatic_recovery_points(2).unwrap(), 1);
        assert!(storage.info().unwrap().warnings.is_empty());
        assert_eq!(store.clips().unwrap().len(), 1);
    }

    #[test]
    fn recovery_cleanup_retains_three_recent_week_and_manual_without_changing_live_data() {
        let scratch = Scratch::new();
        let mut storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let store = storage.open_store().unwrap();
        let clip = store
            .add(
                "Original live record stays unchanged",
                "Original fixture",
                None,
            )
            .unwrap();
        let now = 20 * PROTECT_AGE_MS;
        let old: Vec<_> = (1..=5)
            .map(|i| point(&storage, &store, "before-migration", i * 1000))
            .collect();
        let manual = point(&storage, &store, "manual", 1);
        let recent: Vec<_> = (1..=4)
            .map(|i| point(&storage, &store, "before-update", now - i * 1000))
            .collect();
        let before = fs::read(storage.current.join(DATABASE)).unwrap();
        assert_eq!(storage.cleanup_automatic_recovery_points(now).unwrap(), 5);
        assert!(old.iter().all(|p| !Path::new(&p.path).exists()));
        assert!(recent.iter().all(|p| Path::new(&p.path).exists()));
        assert!(Path::new(&manual.path).exists());
        assert_eq!(fs::read(storage.current.join(DATABASE)).unwrap(), before);
        assert_eq!(store.clip(clip.id).unwrap().content, clip.content);
        // After the seven-day grace period, the newest three survive, regardless of age.
        assert_eq!(
            storage
                .cleanup_automatic_recovery_points(now + PROTECT_AGE_MS)
                .unwrap(),
            1
        );
        assert!(recent[..3].iter().all(|p| Path::new(&p.path).exists()));
        assert!(verified_recovery(&storage.recovery_directory(), &recent[0].id).is_ok());
        assert_eq!(
            storage
                .cleanup_automatic_recovery_points(now + PROTECT_AGE_MS)
                .unwrap(),
            0
        );
    }

    #[test]
    fn recovery_cleanup_preserves_unknown_modified_linked_and_current_archives_and_freezes() {
        let scratch = Scratch::new();
        let mut storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let store = storage.open_store().unwrap();
        store.add("Original protected fixture", "", None).unwrap();
        let now = 20 * PROTECT_AGE_MS;
        let current = point(&storage, &store, "before-migration", 1);
        let altered = point(&storage, &store, "before-migration", 2);
        fs::write(
            Path::new(&altered.path).join("unrecognized.txt"),
            b"original added file",
        )
        .unwrap();
        let damaged = point(&storage, &store, "before-update", 3);
        fs::write(
            Path::new(&damaged.path).join("snapshot.json"),
            b"invalid original manifest",
        )
        .unwrap();
        let future = point(&storage, &store, "before-migration", now + 1);
        let newer_format = point(&storage, &store, "before-migration", 4);
        let (path, mut manifest) =
            read_manifest(&storage.recovery_directory(), &newer_format.id).unwrap();
        manifest.format_version = 2;
        fs::write(
            path.join("snapshot.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        let sized = point(&storage, &store, "before-migration", 5);
        OpenOptions::new()
            .append(true)
            .open(Path::new(&sized.path).join(DATABASE))
            .unwrap()
            .write_all(b"original change")
            .unwrap();
        let healthy: Vec<_> = (10..=13)
            .map(|i| point(&storage, &store, "before-restore", i))
            .collect();
        let real_current = storage.current.clone();
        storage.current = PathBuf::from(&current.path);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                &real_current,
                storage.recovery_directory().join("original-link"),
            )
            .unwrap();
        }
        assert_eq!(storage.cleanup_automatic_recovery_points(now).unwrap(), 1);
        assert!(!Path::new(&healthy[0].path).exists());
        for p in [&current, &altered, &damaged, &future, &newer_format, &sized] {
            assert!(Path::new(&p.path).exists());
        }
        assert!(real_current.join(DATABASE).exists());
        storage.current = real_current;
        storage.freeze_for_update().unwrap();
        assert!(storage.cleanup_automatic_recovery_points(now).is_err());
        assert!(Path::new(&current.path).exists());
    }
    #[test]
    fn recovery_cleanup_reports_failures_without_touching_current_data_and_retries() {
        let scratch = Scratch::new();
        let mut storage = Storage::open(&scratch.0, &scratch.0).unwrap();
        let store = storage.open_store().unwrap();
        let clip = store
            .add("Original data survives cleanup failure", "", None)
            .unwrap();
        fs::write(
            storage.recovery_directory(),
            b"Original invalid root fixture",
        )
        .unwrap();
        assert!(storage
            .cleanup_automatic_recovery_points(PROTECT_AGE_MS)
            .is_err());
        assert!(storage
            .info()
            .unwrap()
            .warnings
            .iter()
            .any(|s| s.contains("旧自动恢复点清理未完成")));
        assert_eq!(store.clip(clip.id).unwrap().content, clip.content);
        fs::remove_file(storage.recovery_directory()).unwrap();
        fs::create_dir(storage.recovery_directory()).unwrap();
        assert_eq!(
            storage
                .cleanup_automatic_recovery_points(PROTECT_AGE_MS)
                .unwrap(),
            0
        );
        assert!(storage.info().unwrap().warnings.is_empty());
        storage.pending = Some(storage.current.clone());
        assert!(storage
            .cleanup_automatic_recovery_points(PROTECT_AGE_MS)
            .is_err());
    }
}
