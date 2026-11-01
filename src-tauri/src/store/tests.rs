use super::*;
fn store() -> Store {
    Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
}

#[test]
fn window_startup_preferences_default_without_writing_and_accept_missing_fields() {
    let db = store();
    assert_eq!(
        db.window_startup_preferences().unwrap(),
        WindowStartupPreferences::default()
    );
    let count: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM preferences WHERE key='window_startup_v1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    for (json, expected) in [
        ("{}", WindowStartupPreferences::default()),
        (
            r#"{"startHidden":true}"#,
            WindowStartupPreferences {
                start_hidden: true,
                show_tray_icon: true,
            },
        ),
        (
            r#"{"showTrayIcon":false}"#,
            WindowStartupPreferences {
                start_hidden: false,
                show_tray_icon: false,
            },
        ),
    ] {
        db.conn.execute(
            "INSERT INTO preferences(key,value) VALUES('window_startup_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]
        ).unwrap();
        assert_eq!(db.window_startup_preferences().unwrap(), expected);
        let saved: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='window_startup_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, json, "reading defaults must not rewrite stored data");
    }
}

#[test]
fn window_startup_preferences_persist_without_touching_capture_or_retention() {
    let scratch = Scratch::new();
    let path = scratch.0.join("window-startup.sqlite3");
    let db = Store::open(&path).unwrap();
    let saved = WindowStartupPreferences {
        start_hidden: true,
        show_tray_icon: false,
    };
    db.save_window_startup_preferences(&saved).unwrap();
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.window_startup_preferences().unwrap(), saved);
    let ordinary = r#"{"paused":false,"pauseRevision":9,"theme":"dark","layout":"grid","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [ordinary],
        )
        .unwrap();
    db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('启动偏好原创超期夹具','text',0,0)", []).unwrap();
    db.conn.execute_batch("CREATE TRIGGER forbid_startup_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'startup must not prune'); END;").unwrap();
    let before = db.settings().unwrap();
    db.save_window_startup_preferences(&WindowStartupPreferences::default())
        .unwrap();
    assert_eq!(db.settings().unwrap(), before);
    assert_eq!(db.clips().unwrap().len(), 1);
    let settings: String = db
        .conn
        .query_row(
            "SELECT value FROM preferences WHERE key='settings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(settings, ordinary);
    let json: String = db
        .conn
        .query_row(
            "SELECT value FROM preferences WHERE key='window_startup_v1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        serde_json::json!({ "startHidden": false, "showTrayIcon": true })
    );
    db.conn.execute_batch("CREATE TRIGGER fail_startup_preferences BEFORE UPDATE ON preferences WHEN NEW.key='window_startup_v1' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(db.save_window_startup_preferences(&saved).is_err());
    assert_eq!(
        db.window_startup_preferences().unwrap(),
        WindowStartupPreferences::default()
    );
}

#[test]
fn window_startup_preferences_report_corruption_without_resetting_it() {
    let db = store();
    for json in [
        "broken",
        "null",
        r#"{"startHidden":"true"}"#,
        r#"{"showTrayIcon":null}"#,
    ] {
        db.conn.execute(
            "INSERT INTO preferences(key,value) VALUES('window_startup_v1',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [json]
        ).unwrap();
        assert!(db.window_startup_preferences().is_err());
        let saved: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='window_startup_v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(saved, json);
    }
}

#[test]
fn history_selected_details_are_bounded_fresh_and_omit_deleted_ids() {
    let store = store();
    let first = store.add("原创详情一", "旧标题", None).unwrap();
    let second = store.add("原创详情二", "待删除", None).unwrap();
    store.update(first.id, "新标题", None, true).unwrap();
    store.set_clip_privacy(first.id, true, false).unwrap();
    store
        .conn
        .execute("DELETE FROM clips WHERE id=?1", [second.id])
        .unwrap();
    let details = store
        .clip_details(&[second.id, first.id, first.id])
        .unwrap();
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].title, "新标题");
    assert!(details[0].is_private);
    assert!(store.clip_details(&[]).unwrap().is_empty());
    assert!(store.clip_details(&[0]).is_err());
    assert!(store.clip_details(&vec![first.id; 501]).is_err());
}

#[test]
fn history_page_bounds_content_and_searches_the_entire_store() {
    let store = store();
    let mut first_id = 0;
    for index in 0..123 {
        let clip = store
            .add(
                &format!("分页原创内容 {index}"),
                if index == 0 {
                    "ÉTOILE 100%_"
                } else {
                    "原创固定样例"
                },
                None,
            )
            .unwrap();
        if index == 0 {
            first_id = clip.id;
        }
    }
    store
        .conn
        .execute("UPDATE clips SET updated_at=100", [])
        .unwrap();
    let first = store.history_page(&HistoryQuery::default()).unwrap();
    assert_eq!(first.clips.len(), 50);
    assert_eq!(first.history.total, 123);
    assert_eq!(first.history.counts["all"], 123);
    assert!(first.clips.windows(2).all(|pair| pair[0].id > pair[1].id));
    let last = store
        .history_page(&HistoryQuery {
            page: u32::MAX,
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(last.history.page, 3);
    assert_eq!(last.clips.len(), 23);
    assert!(!first
        .clips
        .iter()
        .any(|clip| last.clips.iter().any(|other| other.id == clip.id)));
    let search = store
        .history_page(&HistoryQuery {
            query: "étoile 100%_".into(),
            page: 3,
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(search.history.page, 1);
    assert_eq!(search.history.total, 1);
    assert_eq!(search.clips[0].id, first_id);
    let empty = store
        .history_page(&HistoryQuery {
            query: "并不存在的原创文字".into(),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!((empty.history.page, empty.history.total), (1, 0));
    assert_eq!(
        store.clips().unwrap().len(),
        123,
        "paging never prunes retained history"
    );
    store
        .conn
        .execute(
            "DELETE FROM clips WHERE id IN (SELECT id FROM clips ORDER BY id LIMIT 23)",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .history_page(&HistoryQuery {
                page: 3,
                ..HistoryQuery::default()
            })
            .unwrap()
            .history
            .page,
        2,
        "deletion clamps an emptied last page"
    );
}

#[test]
fn history_page_keeps_global_counts_facets_and_privacy_filters() {
    let store = store();
    let group = store.create_group("原创分页分组", "#778899").unwrap();
    let ordinary = store
        .add(
            "https://linux.do/original-page-fixture",
            "原创链接",
            Some(group.id),
        )
        .unwrap();
    store
        .set_clip_tags(ordinary.id, &["原创标签".into()])
        .unwrap();
    store
        .update(ordinary.id, "原创链接", Some(group.id), true)
        .unwrap();
    let private = store.add("测试普通内容", "人工保护样例", None).unwrap();
    store.set_clip_privacy(private.id, true, false).unwrap();
    let image = store
        .add_image(&image_fixture(44), "原创像素", None, false)
        .unwrap();
    let mut legacy = read_metadata(&store.conn, image.id).unwrap();
    legacy.privacy_reasons = vec!["图片尚未进行文字识别".into()];
    put_metadata(&store.conn, image.id, &legacy).unwrap();
    let query = HistoryQuery {
        filter: format!("group-{}", group.id),
        sources: vec!["host:linux.do".into()],
        tag: "原创标签".into(),
        privacy: "ordinary".into(),
        ..HistoryQuery::default()
    };
    let result = store.history_page(&query).unwrap();
    assert_eq!(result.clips.len(), 1);
    assert_eq!(result.clips[0].id, ordinary.id);
    assert_eq!(result.history.counts["all"], 3);
    assert_eq!(result.history.counts["pinned"], 1);
    assert_eq!(result.history.counts["link"], 1);
    assert_eq!(result.history.group_counts[&group.id], 1);
    assert!(result.history.tags.contains(&"原创标签".into()));
    assert!(result.history.hosts.contains(&"linux.do".into()));
    assert!(result.history.sources.contains(&"linuxdo".into()));
    let protected = store
        .history_page(&HistoryQuery {
            privacy: "private".into(),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(
        protected
            .clips
            .iter()
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        [private.id]
    );
    let ai = store
        .history_page(&HistoryQuery {
            ai_eligible: true,
            query: "原创标签".into(),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(
        ai.clips.iter().map(|clip| clip.id).collect::<Vec<_>>(),
        [ordinary.id]
    );
    let pending = store
        .history_page(&HistoryQuery {
            privacy: "pending".into(),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(
        pending.clips.iter().map(|clip| clip.id).collect::<Vec<_>>(),
        [image.id]
    );
}

fn all_cleanup() -> CleanupFilter {
    CleanupFilter {
        min_bytes: 0,
        older_than_days: 0,
        kind: "all".into(),
        include_grouped: false,
    }
}
fn image_fixture(shade: u8) -> PreparedImage {
    images::prepare_rgba(2, 2, &[shade, 70, 180, 255].repeat(4)).unwrap()
}
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "starclip-cleanup-test-{}-{}",
            std::process::id(),
            CLEANUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cleanup_preview_masks_private_titles_without_changing_candidates() {
    let db = store();
    let clip = db
        .add("原创保护预览", "只在主动展开时显示的标题", None)
        .unwrap();
    db.set_clip_privacy(clip.id, true, false).unwrap();
    let preview = db.preview_cleanup(&all_cleanup()).unwrap().preview;
    assert_eq!(preview.matched_count, 1);
    assert_eq!(preview.items[0].id, clip.id);
    assert_eq!(preview.items[0].title, "隐私记录");
    assert_eq!(db.clip(clip.id).unwrap().title, "只在主动展开时显示的标题");
}

#[test]
fn record_identity_survives_merge_but_never_reuses_a_deleted_grant() {
    let mut db = store();
    let original = db.add("记录身份原创夹具", "", None).unwrap();
    assert_eq!(original.record_identity.len(), 64);
    assert_eq!(
        db.add(&original.content, "", None).unwrap().record_identity,
        original.record_identity
    );
    let backup = db.export().unwrap();
    assert!(!backup.contains("recordIdentity"));
    db.import(&backup).unwrap();
    assert_eq!(
        db.clip(original.id).unwrap().record_identity,
        original.record_identity
    );
    db.delete(original.id).unwrap();
    let replacement = db.add("后来新增的另一条记录", "", None).unwrap();
    assert_eq!(
        replacement.id, original.id,
        "fixture must exercise SQLite rowid reuse"
    );
    assert_ne!(replacement.record_identity, original.record_identity);
    let mut imported = store();
    imported.import(&backup).unwrap();
    assert_ne!(
        imported.clips().unwrap()[0].record_identity,
        original.record_identity
    );
}

#[test]
fn privacy_is_independent_of_labels_and_never_downgraded_by_merges() {
    let mut db = store();
    let clip = db.add("原创私密笔记", "", None).unwrap();
    assert!(!clip.is_private);
    assert_eq!(clip.scan_version, 1);
    db.set_clip_privacy(clip.id, true, false).unwrap();
    db.set_clip_tags(clip.id, &["资料".into()]).unwrap();
    let before = db.export().unwrap();
    assert!(db.add("原创私密笔记", "新标题", None).unwrap().is_private);
    db.set_clip_tags(clip.id, &[]).unwrap();
    assert!(db.clip(clip.id).unwrap().is_private);
    assert!(db.set_clip_privacy(clip.id, false, false).is_err());
    let mut untrusted: serde_json::Value = serde_json::from_str(&before).unwrap();
    untrusted["clips"][0]["privacyManual"] = false.into();
    untrusted["clips"][0]["isPrivate"] = false.into();
    untrusted["clips"][0]["privacyReasons"] = serde_json::json!([]);
    untrusted["clips"][0]["scanVersion"] = 999.into();
    db.import(&untrusted.to_string()).unwrap();
    assert!(db.clip(clip.id).unwrap().is_private);
    assert_eq!(db.clip(clip.id).unwrap().scan_version, 1);
    assert!(
        !db.set_clip_privacy(clip.id, false, true)
            .unwrap()
            .is_private
    );
    let sensitive = db.add("password = fixed-fixture-only", "", None).unwrap();
    assert!(sensitive.is_private);
    assert!(
        db.set_clip_privacy(sensitive.id, false, true)
            .unwrap()
            .is_private
    );
    db.save_code(
        Some(sensitive.id),
        "",
        "let safe_fixture = 42;",
        "javascript",
        None,
    )
    .unwrap();
    assert!(
        db.clip(sensitive.id).unwrap().is_private,
        "editing cannot silently remove prior protection"
    );
    assert!(
        !db.set_clip_privacy(sensitive.id, false, true)
            .unwrap()
            .is_private
    );
}

#[test]
fn legacy_image_pending_marker_is_not_privacy_but_actual_protection_survives() {
    let db = store();
    let image = db
        .add_image(&image_fixture(21), "原创截图", None, false)
        .unwrap();
    assert!(!image.is_private);
    assert_eq!(image.scan_version, 0);
    let mut metadata = read_metadata(&db.conn, image.id).unwrap();
    metadata.privacy_reasons = vec!["图片尚未进行文字识别".into()];
    put_metadata(&db.conn, image.id, &metadata).unwrap();
    let old_json: String = db
        .conn
        .query_row(
            "SELECT metadata FROM clip_privacy WHERE clip_id=?1",
            [image.id],
            |row| row.get(0),
        )
        .unwrap();
    let visible = db.clip(image.id).unwrap();
    assert!(!visible.is_private);
    assert!(visible.privacy_reasons.is_empty());
    assert_eq!(
        old_json,
        db.conn
            .query_row::<String, _, _>(
                "SELECT metadata FROM clip_privacy WHERE clip_id=?1",
                [image.id],
                |row| row.get(0)
            )
            .unwrap(),
        "reading does not rewrite user data"
    );
    let mut imported = store();
    let mut old_backup: serde_json::Value = serde_json::from_str(&db.export().unwrap()).unwrap();
    old_backup["clips"][0]["isPrivate"] = true.into();
    old_backup["clips"][0]["privacyReasons"] = serde_json::json!(["图片尚未进行文字识别"]);
    imported.import(&old_backup.to_string()).unwrap();
    assert!(!imported.clips().unwrap()[0].is_private);
    old_backup["clips"][0]["privacyManual"] = true.into();
    imported.import(&old_backup.to_string()).unwrap();
    assert!(imported.clips().unwrap()[0].privacy_manual);
    metadata.privacy_manual = true;
    put_metadata(&db.conn, image.id, &metadata).unwrap();
    assert!(db.clip(image.id).unwrap().is_private);
    metadata.privacy_manual = false;
    metadata.privacy_reasons.push("自定义规则：原创规则".into());
    put_metadata(&db.conn, image.id, &metadata).unwrap();
    assert_eq!(
        db.clip(image.id).unwrap().privacy_reasons,
        vec!["自定义规则：原创规则"]
    );
    assert!(db.clip(image.id).unwrap().is_private);
    db.update(image.id, "api_key=original-fixture-only", None, false)
        .unwrap();
    assert!(db
        .clip(image.id)
        .unwrap()
        .privacy_reasons
        .contains(&"凭据字段".into()));
}

#[test]
fn privacy_rules_rescan_preview_is_readonly_and_stale_application_is_rejected() {
    let db = store();
    let clip = db
        .add("原创内部计划 https://linux.do/t/fixture", "", None)
        .unwrap();
    let mut rules = db.privacy_rules().unwrap();
    rules.rules.push(privacy::PrivacyRule {
        id: "work".into(),
        name: "内部".into(),
        enabled: true,
        condition: "keyword".into(),
        value: "内部".into(),
        tag: Some("工作".into()),
        mark_private: true,
    });
    rules = db.save_privacy_rules(&rules).unwrap();
    assert_eq!(rules.revision, 2);
    assert_eq!(db.clip(clip.id).unwrap().scan_version, 1);
    let preview = db.privacy_rescan_preview().unwrap();
    assert_eq!(preview.newly_private_count, 1);
    assert!(
        !db.clip(clip.id).unwrap().is_private,
        "preview must not persist a scan"
    );
    db.add("预览后的新记录", "", None).unwrap();
    assert!(db.privacy_rescan_apply(&preview.token).is_err());
    let preview = db.privacy_rescan_preview().unwrap();
    db.privacy_rescan_apply(&preview.token).unwrap();
    let scanned = db.clip(clip.id).unwrap();
    assert!(scanned.is_private);
    assert_eq!(scanned.scan_version, 2);
    assert_eq!(scanned.tags, vec!["工作"]);
    assert!(scanned.manual_tags.is_empty());
    db.set_clip_tags(clip.id, &["手动".into()]).unwrap();
    assert_eq!(db.clip(clip.id).unwrap().tags, vec!["工作", "手动"]);
    rules.rules[0].enabled = false;
    let current = db.save_privacy_rules(&rules).unwrap();
    assert!(db.save_privacy_rules(&rules).is_err());
    assert_eq!(current.revision, 3);
    db.privacy_rescan_apply(&db.privacy_rescan_preview().unwrap().token)
        .unwrap();
    assert!(
        db.clip(clip.id).unwrap().is_private,
        "rule removal does not remove protection"
    );
    assert_eq!(db.clip(clip.id).unwrap().tags, vec!["手动"]);
    assert!(
        !db.set_clip_privacy(clip.id, false, true)
            .unwrap()
            .is_private
    );
}

#[test]
fn privacy_backup_v3_roundtrip_reclassifies_and_keeps_imported_rules_disabled() {
    let mut source = store();
    let mut rules = source.privacy_rules().unwrap();
    rules.rules.push(privacy::PrivacyRule {
        id: "work".into(),
        name: "内部".into(),
        enabled: true,
        condition: "keyword".into(),
        value: "内部".into(),
        tag: Some("工作".into()),
        mark_private: true,
    });
    source.save_privacy_rules(&rules).unwrap();
    let original = source
        .add(
            "原创内部记录 https://www.bilibili.com/video/fixture",
            "",
            None,
        )
        .unwrap();
    let manual = source.add("原创手动保护记录", "", None).unwrap();
    source.set_clip_privacy(manual.id, true, false).unwrap();
    source.set_clip_tags(manual.id, &["手动".into()]).unwrap();
    let json = source.export().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["version"],
        3
    );
    let mut destination = store();
    destination.import(&json).unwrap();
    let imported = destination
        .clips()
        .unwrap()
        .into_iter()
        .find(|clip| clip.content == original.content)
        .unwrap();
    assert!(imported.is_private);
    assert!(
        imported.tags.is_empty(),
        "automatic tags are recomputed under local active rules"
    );
    assert_eq!(imported.link_sources, vec!["bilibili"]);
    assert_eq!(
        imported.scan_version,
        destination.privacy_rules().unwrap().revision,
        "imported scan version must not be trusted"
    );
    let local_rules = destination.privacy_rules().unwrap();
    assert_eq!(local_rules.rules.len(), 1);
    assert!(!local_rules.rules[0].enabled);
    assert!(local_rules.rules[0].id.starts_with("import-"));
    destination.import(&json).unwrap();
    assert_eq!(
        destination.privacy_rules().unwrap().rules.len(),
        1,
        "reimport must not duplicate drafts"
    );
    let imported_manual = destination
        .clips()
        .unwrap()
        .into_iter()
        .find(|clip| clip.content == manual.content)
        .unwrap();
    assert!(imported_manual.privacy_manual);
    assert_eq!(imported_manual.manual_tags, vec!["手动"]);
    // Real v1 input omits all privacy metadata; it still goes through the detector.
    let legacy = serde_json::json!({"format":"starclip","version":1,"exportedAt":0,"groups":[],"clips":[{"id":99,"content":"https://fixture.test/?api_key=fixture","title":"","kind":"link","pinned":false,"groupId":null,"createdAt":0,"updatedAt":0,"copyCount":0}]});
    source.import(&legacy.to_string()).unwrap();
    assert!(
        source
            .clips()
            .unwrap()
            .iter()
            .find(|clip| clip.content.starts_with("https://fixture"))
            .unwrap()
            .is_private
    );
}

#[test]
fn privacy_migration_scans_v1_and_full_snapshot_preserves_metadata() {
    let scratch = Scratch::new();
    let path = scratch.0.join("privacy-upgrade.sqlite3");
    let db = Store::open(&path).unwrap();
    let clip = db.add("password=original-fixed-fixture", "", None).unwrap();
    db.set_clip_tags(clip.id, &["备份测试".into()]).unwrap();
    let snapshot = scratch.0.join("snapshot.sqlite3");
    db.snapshot(&snapshot).unwrap();
    let restored = Store::open(&snapshot).unwrap();
    assert!(restored.clip(clip.id).unwrap().is_private);
    assert_eq!(
        restored.clip(clip.id).unwrap().manual_tags,
        vec!["备份测试"]
    );
    drop(restored);
    drop(db);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE clip_privacy; PRAGMA user_version=1;")
        .unwrap();
    drop(conn);
    let upgraded = Store::open(&path).unwrap();
    assert_eq!(upgraded.schema_version().unwrap(), 2);
    assert!(upgraded.clip(clip.id).unwrap().is_private);
    assert_eq!(upgraded.clip(clip.id).unwrap().scan_version, 1);
    assert_eq!(fs::read_dir(scratch.0.join("recovery")).unwrap().count(), 1);
}

#[test]
fn privacy_failure_rolls_back_capture_import_code_and_schema_upgrade() {
    let mut db = store();
    let code = db
        .save_code(None, "安全代码", "const fixture = 7;", "javascript", None)
        .unwrap();
    let old = db.export().unwrap();
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('privacy_rules_v1','broken')",
            [],
        )
        .unwrap();
    assert!(db.add("不能半写入的记录", "", None).is_err());
    assert!(db
        .save_code(
            Some(code.id),
            "新标题",
            "const fixture = 8;",
            "javascript",
            None
        )
        .is_err());
    assert!(db.import(&old).is_err());
    assert_eq!(db.clips().unwrap().len(), 1);
    assert_eq!(db.clip(code.id).unwrap().content, "const fixture = 7;");
    let scratch = Scratch::new();
    let path = scratch.0.join("rollback.sqlite3");
    let store = Store::open(&path).unwrap();
    store.add("升级回滚原创夹具", "", None).unwrap();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE clip_privacy; PRAGMA user_version=1; INSERT INTO preferences(key,value) VALUES('privacy_rules_v1','broken');").unwrap();
    drop(conn);
    let before = fs::read(&path).unwrap();
    assert!(Store::open(&path).err().unwrap().contains("恢复点已保留"));
    assert_eq!(fs::read(&path).unwrap(), before);
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(Store::check_schema_version(&conn).unwrap(), 1);
    assert!(conn.prepare("SELECT * FROM clip_privacy").is_err());
}

#[test]
fn appearance_is_independent_atomic_persistent_and_in_full_snapshots() {
    let scratch = Scratch::new();
    let path = scratch.0.join("appearance.sqlite3");
    let db = Store::open(&path).unwrap();
    assert_eq!(db.appearance().unwrap(), Appearance::default());
    assert_eq!(
        db.conn
            .query_row(
                "SELECT count(*) FROM preferences WHERE key='appearance_v1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let png = images::prepare_rgba(2, 1, &[80, 120, 200, 0, 160, 80, 220, 255]).unwrap();
    let base64 = STANDARD.encode(&png.png);
    db.save_appearance(
        &serde_json::json!({"accent":"#AABBCC", "background":"gradient", "angle":45}),
    )
    .unwrap();
    let saved = db
        .import_appearance_sticker(&base64, "原创小星星.png")
        .unwrap();
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.appearance().unwrap(), saved);
    let snapshot = scratch.0.join("appearance-snapshot.sqlite3");
    db.snapshot(&snapshot).unwrap();
    assert_eq!(Store::open(&snapshot).unwrap().appearance().unwrap(), saved);
    let ordinary = r#"{"paused":false,"theme":"dark","layout":"horizontal","maxHistory":1,"shortcut":"CommandOrControl+Shift+V","retentionDays":1,"futurePreference":"preserve"}"#;
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [ordinary],
        )
        .unwrap();
    db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('外观测试中的原创过期记录','text',0,0)", []).unwrap();
    db.conn.execute_batch("CREATE TRIGGER forbid_appearance_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'appearance must not prune'); END;").unwrap();
    let disabled = db
        .save_appearance(&serde_json::json!({"stickerEnabled":false, "colorEnd":"#FFFCDC"}))
        .unwrap();
    assert!(!disabled.sticker_enabled);
    assert_eq!(disabled.sticker_data_url, saved.sticker_data_url);
    assert_eq!(
        db.conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        ordinary
    );
    assert_eq!(db.clips().unwrap().len(), 1);
    assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
    assert!(!db.export().unwrap().contains("appearance_v1"));
    assert!(db
        .import_appearance_sticker("invalid", "新贴纸.png")
        .is_err());
    assert!(db
        .save_appearance(&serde_json::json!({"angle":360}))
        .is_err());
    assert_eq!(db.appearance().unwrap(), disabled);
    db.conn.execute_batch("CREATE TRIGGER fail_appearance_write BEFORE UPDATE ON preferences WHEN NEW.key='appearance_v1' BEGIN SELECT RAISE(ABORT, 'fixture disk error'); END;").unwrap();
    assert!(db.import_appearance_sticker(&base64, "替换图.png").is_err());
    assert!(db.remove_appearance_sticker().is_err());
    assert_eq!(db.appearance().unwrap(), disabled);
    db.conn
        .execute_batch("DROP TRIGGER fail_appearance_write;")
        .unwrap();
    let removed = db.remove_appearance_sticker().unwrap();
    assert_eq!(removed.accent, disabled.accent);
    assert!(!removed.sticker_enabled);
    assert!(removed.sticker_data_url.is_none());
    assert_eq!(db.clips().unwrap().len(), 1);
    db.conn
        .execute(
            "UPDATE preferences SET value='{}' WHERE key='appearance_v1'",
            [],
        )
        .unwrap();
    assert!(db.appearance().is_err());
    assert!(db
        .save_appearance(&serde_json::json!({"angle":90}))
        .is_err());
    assert_eq!(
        db.conn
            .query_row(
                "SELECT value FROM preferences WHERE key='appearance_v1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "{}"
    );
}

#[test]
fn starpet_growth_is_atomic_independent_and_revisioned() {
    use crate::starpet_growth::{encode_save, GrowthChange, GrowthState};
    let scratch = Scratch::new();
    let path = scratch.0.join("growth.sqlite3");
    let db = Store::open(&path).unwrap();
    let preferences = StarpetPreferences {
        enabled: true,
        mood: "reading".into(),
        color: "#AADDEE".into(),
        size: 140,
    };
    db.save_starpet_preferences(&preferences).unwrap();
    let settings = db.settings().unwrap();
    assert!(settings.paused);
    assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
    assert_eq!(
        db.conn
            .query_row(
                "SELECT count(*) FROM preferences WHERE key='starpet_growth_v1'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    db.conn.execute_batch("CREATE TRIGGER growth_must_not_touch_clips BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'must not prune'); END;").unwrap();
    let today = "2026-10-26";
    let enabled = db
        .change_starpet_growth_on(GrowthChange::Enabled(true), today)
        .unwrap();
    assert_eq!(enabled.revision, 1);
    let first = db
        .change_starpet_growth_on(GrowthChange::Interaction("greet"), today)
        .unwrap();
    assert_eq!(first.revision, 2);
    for _ in 0..5 {
        assert_eq!(
            db.change_starpet_growth_on(GrowthChange::Interaction("greet"), today)
                .unwrap(),
            first
        );
    }
    let second = db
        .change_starpet_growth_on(GrowthChange::Interaction("highfive"), today)
        .unwrap();
    assert_eq!((second.xp, second.revision), (25, 3));
    assert_eq!(db.starpet_preferences().unwrap(), preferences);
    assert_eq!(db.settings().unwrap(), settings);
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.starpet_growth().unwrap(), second);
    assert_eq!(db.starpet_preferences().unwrap(), preferences);
    let saved = encode_save(second.clone()).unwrap();
    let grown = db
        .change_starpet_growth_on(GrowthChange::Interaction("stretch"), today)
        .unwrap();
    assert!(db
        .change_starpet_growth_on(
            GrowthChange::Import {
                data: &saved,
                expected_revision: second.revision
            },
            today
        )
        .is_err());
    assert_eq!(db.starpet_growth().unwrap(), grown);
    let restored = db
        .change_starpet_growth_on(
            GrowthChange::Import {
                data: &saved,
                expected_revision: grown.revision,
            },
            today,
        )
        .unwrap();
    assert_eq!((restored.xp, restored.revision), (25, grown.revision + 1));
    db.conn.execute_batch("CREATE TRIGGER fail_growth_write BEFORE UPDATE ON preferences WHEN NEW.key='starpet_growth_v1' BEGIN SELECT RAISE(ABORT, 'fixture disk error'); END;").unwrap();
    assert!(db
        .change_starpet_growth_on(GrowthChange::Interaction("stretch"), today)
        .is_err());
    assert_eq!(db.starpet_growth().unwrap(), restored);
    db.conn
        .execute_batch("DROP TRIGGER fail_growth_write;")
        .unwrap();
    let reset = db
        .change_starpet_growth_on(
            GrowthChange::Reset {
                expected_revision: restored.revision,
            },
            today,
        )
        .unwrap();
    assert_eq!(
        reset,
        GrowthState {
            revision: restored.revision + 1,
            ..GrowthState::default()
        }
    );
    assert_eq!(db.settings().unwrap(), settings);
    assert_eq!(db.starpet_preferences().unwrap(), preferences);
}

#[test]
fn starpet_preferences_are_optional_persistent_and_independent_of_capture_and_retention() {
    let scratch = Scratch::new();
    let path = scratch.0.join("starpet.sqlite3");
    let fresh = Store::open(&path).unwrap();
    assert_eq!(
        fresh.starpet_preferences().unwrap(),
        StarpetPreferences::default()
    );
    assert_eq!(
        fresh
            .conn
            .query_row(
                "SELECT count(*) FROM preferences WHERE key='starpet_v1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    fresh
        .conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('starpet_v1','{}')",
            [],
        )
        .unwrap();
    assert_eq!(
        fresh.starpet_preferences().unwrap(),
        StarpetPreferences::default()
    );
    assert_eq!(
        fresh
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='starpet_v1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "{}",
        "reading defaults must not rewrite existing data"
    );
    let legacy = r##"{"enabled":true,"mood":"sleepy"}"##;
    fresh
        .conn
        .execute(
            "UPDATE preferences SET value=?1 WHERE key='starpet_v1'",
            [legacy],
        )
        .unwrap();
    assert_eq!(
        fresh.starpet_preferences().unwrap(),
        StarpetPreferences {
            enabled: true,
            mood: "sleepy".into(),
            color: "#FFD66E".into(),
            size: 100,
        }
    );
    assert_eq!(
        fresh
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='starpet_v1'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        legacy
    );
    let preferences = StarpetPreferences {
        enabled: true,
        mood: "curious".into(),
        color: "#8CB9BD".into(),
        size: 120,
    };
    fresh.save_starpet_preferences(&preferences).unwrap();
    drop(fresh);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.starpet_preferences().unwrap(), preferences);

    let settings = Settings {
        paused: false,
        retention_days: 1,
        max_history: 50,
        ..Settings::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [&json],
        )
        .unwrap();
    db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('星伴表情变化不能删除这条原创超期片段','text',0,0)", []).unwrap();
    db.conn.execute_batch("CREATE TRIGGER forbid_starpet_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'starpet must not prune'); END;").unwrap();
    assert_eq!(STARPET_MOODS.len(), 32);
    for mood in STARPET_MOODS {
        let value = StarpetPreferences {
            enabled: *mood != "sleeping",
            mood: (*mood).into(),
            color: preferences.color.clone(),
            size: preferences.size,
        };
        db.save_starpet_preferences(&value).unwrap();
        assert_eq!(db.starpet_preferences().unwrap(), value);
    }
    assert_eq!(db.settings().unwrap(), settings);
    assert_eq!(db.clips().unwrap().len(), 1);
    assert!(!db.onboarding_completed().unwrap());
    assert_eq!(
        db.conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        json
    );
    let old = db.starpet_preferences().unwrap();
    for mood in ["", "unknown", "IDLE", "idle "] {
        assert!(db
            .save_starpet_preferences(&StarpetPreferences {
                enabled: true,
                mood: mood.into(),
                color: old.color.clone(),
                size: old.size,
            })
            .is_err());
        assert_eq!(db.starpet_preferences().unwrap(), old);
    }
    for (patch, expected) in [
        (
            r#"{"enabled":false}"#,
            StarpetPreferences {
                enabled: false,
                ..old.clone()
            },
        ),
        (
            r#"{"mood":"greet"}"#,
            StarpetPreferences {
                enabled: false,
                mood: "greet".into(),
                color: old.color.clone(),
                size: old.size,
            },
        ),
        (
            r##"{"color":"#aabbcc"}"##,
            StarpetPreferences {
                enabled: false,
                mood: "greet".into(),
                color: "#AABBCC".into(),
                size: old.size,
            },
        ),
        (
            r#"{"size":160}"#,
            StarpetPreferences {
                enabled: false,
                mood: "greet".into(),
                color: "#AABBCC".into(),
                size: 160,
            },
        ),
    ] {
        let patch: StarpetPreferencePatch = serde_json::from_str(patch).unwrap();
        let mut value = db.starpet_preferences().unwrap();
        patch.apply_to(&mut value);
        db.save_starpet_preferences(&value).unwrap();
        assert_eq!(db.starpet_preferences().unwrap(), expected);
    }
    let old = db.starpet_preferences().unwrap();
    for size in [0, 69, 161, u16::MAX] {
        assert!(db
            .save_starpet_preferences(&StarpetPreferences {
                size,
                ..old.clone()
            })
            .is_err());
        assert_eq!(db.starpet_preferences().unwrap(), old);
    }
    for json in [r#"{"size":-1}"#, r#"{"size":100.5}"#, r#"{"size":65536}"#] {
        assert!(serde_json::from_str::<StarpetPreferencePatch>(json).is_err());
    }
    assert_eq!(db.settings().unwrap(), settings);
    assert_eq!(
        db.clips().unwrap().len(),
        1,
        "size changes must not run retention"
    );
    for color in [
        "",
        "#fff",
        "112233",
        "#1234567",
        "#12345g",
        "#１２３",
        " #123456",
        "#123456\n",
    ] {
        assert!(db
            .save_starpet_preferences(&StarpetPreferences {
                color: color.into(),
                ..old.clone()
            })
            .is_err());
        assert_eq!(db.starpet_preferences().unwrap(), old);
    }
    db.conn.execute_batch("CREATE TRIGGER fail_starpet_save BEFORE UPDATE ON preferences WHEN NEW.key='starpet_v1' BEGIN SELECT RAISE(ABORT, 'fixture disk error'); END;").unwrap();
    assert!(db.save_starpet_preferences(&preferences).is_err());
    assert_eq!(
        db.starpet_preferences().unwrap(),
        old,
        "failed persistence retains the complete previous preference"
    );
}

#[test]
fn starpet_geometry_anchors_bottom_center_and_stays_in_work_area() {
    for (size, expected) in [
        (70, (176.0, 206.0)),
        (100, (230.0, 260.0)),
        (160, (338.0, 368.0)),
    ] {
        let preferences = StarpetPreferences {
            size,
            ..StarpetPreferences::default()
        };
        assert!(preferences.validate().is_ok());
        assert_eq!(preferences.window_size(), expected);
    }
    let area = (0.0, 0.0, 1920.0, 1080.0);
    let enlarged = starpet_resize_position((500.0, 500.0), (230.0, 260.0), (338.0, 368.0), area);
    assert_eq!(enlarged, (446.0, 392.0));
    assert_eq!(
        starpet_resize_position(enlarged, (338.0, 368.0), (230.0, 260.0), area),
        (500.0, 500.0)
    );
    assert_eq!(
        starpet_resize_position((1690.0, 820.0), (230.0, 260.0), (338.0, 368.0), area),
        (1582.0, 712.0)
    );
    assert_eq!(
        starpet_resize_position(
            (-1900.0, -180.0),
            (230.0, 260.0),
            (338.0, 368.0),
            (-1920.0, -200.0, 1920.0, 1080.0)
        ),
        (-1920.0, -200.0)
    );
    assert_eq!(
        starpet_resize_position(
            (1000.0, 1000.0),
            (460.0, 520.0),
            (676.0, 736.0),
            (0.0, 0.0, 3840.0, 2160.0)
        ),
        (892.0, 784.0),
        "geometry uses physical pixels consistently on HiDPI displays"
    );
    assert_eq!(
        starpet_resize_position(
            (20.0, 30.0),
            (10.0, 10.0),
            (200.0, 200.0),
            (10.0, 20.0, 100.0, 100.0)
        ),
        (10.0, 20.0)
    );
}

#[test]
fn starpet_docks_only_near_an_edge_and_keeps_the_other_axis() {
    let area = (0.0, 25.0, 1920.0, 1055.0);
    let size = (230.0, 260.0);
    assert_eq!(
        starpet_dock_position((800.0, 400.0), size, area, 28.0),
        None
    );
    assert_eq!(
        starpet_dock_position((12.0, 400.0), size, area, 28.0),
        Some(("left", 0.0, 400.0))
    );
    assert_eq!(
        starpet_dock_position((1675.0, 400.0), size, area, 28.0),
        Some(("right", 1690.0, 400.0))
    );
    assert_eq!(
        starpet_dock_position((900.0, 30.0), size, area, 28.0),
        Some(("top", 900.0, 25.0))
    );
    assert_eq!(
        starpet_dock_position((900.0, 810.0), size, area, 28.0),
        Some(("bottom", 900.0, 820.0))
    );
    // Dragged partly off screen: snap back onto the nearest edge.
    assert_eq!(
        starpet_dock_position((-60.0, 300.0), size, area, 28.0),
        Some(("left", 0.0, 300.0))
    );
    // In a corner the side wall wins a tie, and the other axis is clamped on screen.
    assert_eq!(
        starpet_dock_position((1700.0, 830.0), size, area, 28.0),
        Some(("right", 1690.0, 820.0))
    );
    assert_eq!(
        starpet_dock_position((-1910.0, 500.0), size, (-1920.0, 0.0, 1920.0, 1080.0), 28.0),
        Some(("left", -1920.0, 500.0)),
        "secondary displays left of the primary use negative coordinates"
    );
}

#[test]
fn telemetry_preferences_persist_without_retention_or_other_preferences() {
    let scratch = Scratch::new();
    let path = scratch.0.join("telemetry-fixture.sqlite3");
    let db = Store::open(&path).unwrap();
    assert_eq!(
        db.telemetry_preferences().unwrap(),
        TelemetryPreferences::default()
    );
    let enabled = db.set_telemetry_enabled(true).unwrap();
    assert!(enabled.enabled);
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.telemetry_preferences().unwrap(), enabled);
    let ordinary = r#"{"paused":false,"theme":"dark","layout":"grid","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [ordinary],
        )
        .unwrap();
    db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('统计偏好原创超期夹具','text',0,0)", []).unwrap();
    db.conn.execute_batch("CREATE TRIGGER forbid_telemetry_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'telemetry must not prune'); END;").unwrap();
    let disabled = db.set_telemetry_enabled(false).unwrap();
    assert!(!disabled.enabled);
    assert!(disabled.revision > enabled.revision);
    let saved: String = db
        .conn
        .query_row(
            "SELECT value FROM preferences WHERE key='settings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, ordinary);
    assert_eq!(db.clips().unwrap().len(), 1);
    assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
    assert_eq!(
        db.update_preferences().unwrap(),
        UpdatePreferences::default()
    );
    let saved: String = db
        .conn
        .query_row(
            "SELECT value FROM preferences WHERE key='telemetry_preferences_v1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        json.as_object().unwrap().len(),
        2,
        "no session ID or event queue persisted"
    );
    db.conn.execute_batch("CREATE TRIGGER fail_telemetry_preferences BEFORE UPDATE ON preferences WHEN NEW.key='telemetry_preferences_v1' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(db.set_telemetry_enabled(true).is_err());
    assert_eq!(db.telemetry_preferences().unwrap(), disabled);
}

#[test]
fn telemetry_default_only_applies_to_missing_preferences_not_opt_out_or_corruption() {
    let scratch = Scratch::new();
    let path = scratch.0.join("telemetry-default-fixture.sqlite3");
    let db = Store::open(&path).unwrap();
    assert!(db.telemetry_preferences().unwrap().enabled);
    let opted_out = db.set_telemetry_enabled(false).unwrap();
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.telemetry_preferences().unwrap(), opted_out);
    assert!(!db.telemetry_preferences().unwrap().enabled);
    for json in [
        "broken",
        r#"{"enabled":true}"#,
        r#"{"revision":0,"enabled":"true"}"#,
        r#"{"revision":18446744073709551615,"enabled":true}"#,
    ] {
        db.conn
            .execute(
                "UPDATE preferences SET value=?1 WHERE key='telemetry_preferences_v1'",
                [json],
            )
            .unwrap();
        assert!(
            db.telemetry_preferences().is_err(),
            "corruption must not act like a missing preference"
        );
    }
    assert!(
        !db.set_telemetry_enabled(false).unwrap().enabled,
        "explicit local choice can repair damaged preferences"
    );
    db.conn.execute_batch("DROP TABLE preferences").unwrap();
    assert!(
        db.telemetry_preferences().is_err(),
        "database read failures must not use the default"
    );
}

#[test]
fn update_preferences_are_independent_persistent_and_daily_attempts_survive_failures() {
    let scratch = Scratch::new();
    let path = scratch.0.join("update-fixture.sqlite3");
    let db = Store::open(&path).unwrap();
    assert_eq!(
        db.update_preferences().unwrap(),
        UpdatePreferences::default()
    );
    let now = 1_000_000;
    let (attempt, started) = db.record_update_check(now, true).unwrap();
    assert!(started);
    assert_eq!(attempt.last_check_at, Some(now));
    assert!(attempt.revision > 0);
    // No completion is recorded: failure and app exit still throttle retries.
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(
        db.record_update_check(now + 1, true).unwrap(),
        (attempt.clone(), false)
    );
    assert!(
        db.record_update_check(now + AUTO_UPDATE_INTERVAL_MS, true)
            .unwrap()
            .1
    );
    let last = db.update_preferences().unwrap();
    let (rebased, started) = db.record_update_check(now, true).unwrap();
    assert!(!started);
    assert!(rebased.revision > last.revision);
    assert_eq!(rebased.last_check_at, Some(now));
    assert!(!db.record_update_check(now + 1, true).unwrap().1);

    let disabled = db.set_auto_update_check(false).unwrap();
    assert!(!disabled.auto_check);
    assert_eq!(
        db.record_update_check(now + 2 * AUTO_UPDATE_INTERVAL_MS, true)
            .unwrap(),
        (disabled, false)
    );
    assert!(
        db.record_update_check(now + 2, false).unwrap().1,
        "manual checks ignore auto setting and interval"
    );
    let skipped = db.skip_update_version("0.3.0").unwrap();
    assert_eq!(skipped.skipped_version.as_deref(), Some("0.3.0"));
    assert!(!skipped.auto_check);
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.update_preferences().unwrap(), skipped);

    let ordinary = r#"{"paused":false,"theme":"dark","layout":"grid","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1}"#;
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [ordinary],
        )
        .unwrap();
    db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('更新偏好原创超期夹具','text',0,0)", []).unwrap();
    db.conn.execute_batch("CREATE TRIGGER forbid_update_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'updates must not prune'); END;").unwrap();
    db.set_auto_update_check(true).unwrap();
    db.skip_update_version("0.4.0").unwrap();
    db.record_update_check(now + 3, false).unwrap();
    let saved: String = db
        .conn
        .query_row(
            "SELECT value FROM preferences WHERE key='settings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, ordinary);
    assert_eq!(db.clips().unwrap().len(), 1);
    assert_eq!(db.starpet_growth().unwrap(), GrowthState::default());
    let before = db.update_preferences().unwrap();
    db.conn.execute_batch("CREATE TRIGGER fail_update_preferences BEFORE UPDATE ON preferences WHEN NEW.key='update_preferences_v1' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(db.set_auto_update_check(false).is_err());
    assert!(db.record_update_check(now + 4, false).is_err());
    assert_eq!(db.update_preferences().unwrap(), before);
}

#[test]
fn immediate_theme_change_preserves_legacy_preferences_and_never_prunes() {
    let scratch = Scratch::new();
    let path = scratch.0.join("theme.sqlite3");
    let fresh = Store::open(&path).unwrap();
    assert_eq!(fresh.settings().unwrap(), Settings::default());
    let initial = fresh.set_theme("white").unwrap();
    assert_eq!(
        initial,
        Settings {
            theme: "white".into(),
            ..Settings::default()
        }
    );
    assert!(!fresh.onboarding_completed().unwrap());
    drop(fresh);
    assert_eq!(Store::open(&path).unwrap().settings().unwrap(), initial);
    let db = store();
    let legacy = r#"{"paused":false,"theme":"dark","layout":"horizontal","maxHistory":50,"shortcut":"CommandOrControl+Shift+V","retentionDays":1,"futurePreference":"preserve"}"#;
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [legacy],
        )
        .unwrap();
    db.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('只换主题不能清理这条原创超期记录','text',0,0)", []).unwrap();
    db.conn.execute_batch("CREATE TRIGGER forbid_theme_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'theme must not prune'); END;").unwrap();
    let mut expected: serde_json::Value = serde_json::from_str(legacy).unwrap();
    for theme in ["light", "white", "forest", "rose", "system", "dark"] {
        let settings = db.set_theme(theme).unwrap();
        assert_eq!(settings.theme, theme);
        assert!(!settings.paused);
        assert_eq!(settings.retention_days, 1);
        assert_eq!(db.clips().unwrap().len(), 1);
        expected["theme"] = theme.into();
        let saved: String = db
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
            expected
        );
    }
    let before = db.settings().unwrap();
    assert!(db.set_theme("unknown").is_err());
    assert_eq!(db.settings().unwrap(), before);
    db.conn.execute_batch("CREATE TRIGGER fail_theme_save BEFORE UPDATE ON preferences WHEN NEW.key='settings' BEGIN SELECT RAISE(ABORT, 'fixture write failure'); END;").unwrap();
    assert!(db.set_theme("rose").is_err());
    assert_eq!(db.settings().unwrap(), before);
    assert_eq!(db.clips().unwrap().len(), 1);
    assert_eq!(
        db.starpet_preferences().unwrap(),
        StarpetPreferences::default()
    );
}

#[test]
fn onboarding_is_upgrade_safe_persistent_and_has_no_retention_or_settings_side_effects() {
    let scratch = Scratch::new();
    let path = scratch.0.join("onboarding.sqlite3");
    let fresh = Store::open(&path).unwrap();
    assert!(!fresh.onboarding_completed().unwrap());
    drop(fresh);
    let fresh = Store::open(&path).unwrap();
    assert!(
        !fresh.onboarding_completed().unwrap(),
        "unfinished first-run status survives restart"
    );
    fresh.complete_onboarding().unwrap();
    drop(fresh);
    let reopened = Store::open(&path).unwrap();
    assert!(reopened.onboarding_completed().unwrap());
    assert_eq!(reopened.settings().unwrap(), Settings::default());
    assert_eq!(
        reopened
            .conn
            .query_row(
                "SELECT count(*) FROM preferences WHERE key='settings'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );

    let old_empty = store();
    old_empty
        .conn
        .execute("DELETE FROM preferences WHERE key='onboarding_v1'", [])
        .unwrap();
    let old_empty = Store::from_connection(old_empty.conn).unwrap();
    assert!(
        old_empty.onboarding_completed().unwrap(),
        "an existing empty database is still an upgrade"
    );

    let existing = store();
    existing
        .conn
        .execute("DELETE FROM preferences WHERE key='onboarding_v1'", [])
        .unwrap();
    let settings = Settings {
        paused: false,
        max_history: 50,
        retention_days: 1,
        ..Settings::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    existing
        .conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1)",
            [&json],
        )
        .unwrap();
    existing.conn.execute("INSERT INTO clips(content,kind,created_at,updated_at) VALUES('引导操作不得清理这条超期片段','text',0,0)", []).unwrap();
    existing.conn.execute_batch("CREATE TRIGGER forbid_onboarding_prune BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'onboarding must not prune'); END;").unwrap();
    assert!(existing.onboarding_completed().unwrap());
    existing.complete_onboarding().unwrap();
    assert!(existing.onboarding_completed().unwrap());
    assert_eq!(existing.clips().unwrap().len(), 1);
    assert_eq!(existing.settings().unwrap(), settings);
    assert_eq!(
        existing
            .conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        json
    );
}

#[test]
fn unlimited_history_and_new_protected_records_do_not_consume_capacity() {
    let db = store();
    assert_eq!(db.settings().unwrap().max_history, 0);
    assert_eq!(db.settings().unwrap().retention_days, 0);
    let mut bytes = 0;
    for index in 0..550 {
        let content = format!("不限历史：{index}");
        bytes += content.len() as u64;
        db.add(&content, "", None).unwrap();
    }
    assert_eq!(db.clips().unwrap().len(), 550);
    db.conn
        .execute("UPDATE clips SET updated_at=0", [])
        .unwrap();
    assert_eq!(db.prune_history().unwrap(), 0);
    assert_eq!(
        db.clips().unwrap().len(),
        550,
        "defaults do not expire old history"
    );
    let preview = db.preview_cleanup(&all_cleanup()).unwrap().preview;
    assert_eq!(preview.matched_count, 550);
    assert_eq!(preview.content_bytes, bytes);
    assert_eq!(preview.items.len(), 100);
    assert!(preview.truncated);
    assert!(preview
        .items
        .windows(2)
        .all(|items| items[0].bytes >= items[1].bytes));
    let mut settings = db.settings().unwrap();
    settings.max_history = 50;
    db.save_settings(&settings).unwrap();
    let ordinary: Vec<_> = db
        .clips()
        .unwrap()
        .into_iter()
        .map(|clip| clip.id)
        .collect();
    let keep = db
        .add_protected("新增时即永久保留", "", None, true)
        .unwrap();
    assert!(keep.keep_forever);
    assert_eq!(db.clips().unwrap().len(), 51);
    for id in ordinary {
        assert!(db.clip(id).is_ok());
    }
    settings.max_history = 0;
    settings.retention_days = 1;
    db.conn
        .execute("UPDATE clips SET updated_at=0", [])
        .unwrap();
    db.save_settings(&settings).unwrap();
    assert_eq!(
        db.clips().unwrap().len(),
        1,
        "unlimited count does not disable the chosen age policy"
    );
    assert_eq!(db.clips().unwrap()[0].id, keep.id);
}

#[test]
fn permanent_protection_survives_every_mutation_and_backup_merge() {
    let mut db = store();
    let keep = db
        .add_protected("保留原创片段", "初稿", None, true)
        .unwrap();
    let pinned = db.add("星标片段", "", None).unwrap();
    db.update(pinned.id, "", None, true).unwrap();
    assert!(db.add("保留原创片段", "备注", None).unwrap().keep_forever);
    db.save_code(Some(keep.id), "代码", "println!(\"星光\");", "rust", None)
        .unwrap();
    db.update(keep.id, "取消星标仍保留", None, false).unwrap();
    assert!(db.clip(keep.id).unwrap().keep_forever);
    assert!(db.delete(keep.id).unwrap_err().contains("先取消保留"));
    db.add("可以清理的片段", "", None).unwrap();
    let mut backup: serde_json::Value = serde_json::from_str(&db.export().unwrap()).unwrap();
    for clip in backup["clips"].as_array_mut().unwrap() {
        clip["keepForever"] = false.into();
    }
    db.import(&backup.to_string()).unwrap();
    assert!(
        db.clip(keep.id).unwrap().keep_forever,
        "import cannot downgrade existing protection"
    );
    let mut restored = store();
    restored.import(&db.export().unwrap()).unwrap();
    assert!(restored
        .clips()
        .unwrap()
        .iter()
        .any(|clip| clip.keep_forever));
    db.clear_history().unwrap();
    assert_eq!(db.clips().unwrap().len(), 2);
    let preview = db.preview_cleanup(&all_cleanup()).unwrap();
    assert_eq!(preview.preview.protected_count, 2);
    assert_eq!(preview.preview.matched_count, 0);
    assert_eq!(db.execute_cleanup(preview, false).unwrap().deleted_count, 0);
    db.set_keep_forever(keep.id, false).unwrap();
    db.delete(keep.id).unwrap();
    assert!(db.clip(keep.id).is_err());
}

#[test]
fn cleanup_filters_are_utf8_strict_and_combined_and_preview_is_read_only() {
    let db = store();
    db.add("星", "月", None).unwrap(); // 6 UTF-8 bytes
    db.add("abcd", "ef", None).unwrap(); // exactly the same byte threshold
    let large = db.add("银河", "夜", None).unwrap(); // 9 bytes
    let link = db.add("https://example.com", "", None).unwrap();
    let group = db.create_group("清理测试分组", "#aabbcc").unwrap();
    db.add("分组内容", "", Some(group.id)).unwrap();
    let keep = db.add_protected("保留内容", "", None, true).unwrap();
    db.update(keep.id, "", None, true).unwrap(); // overlapping protection counts once
    let pin = db.add("星标内容", "", None).unwrap();
    db.update(pin.id, "", None, true).unwrap();
    db.conn
        .execute("UPDATE clips SET updated_at=0", [])
        .unwrap();
    db.add("fresh text", "", None).unwrap();
    let changes = db.conn.total_changes();
    let mut filter = CleanupFilter {
        min_bytes: 6,
        older_than_days: 1,
        kind: "text".into(),
        include_grouped: false,
    };
    let preview = db.preview_cleanup(&filter).unwrap().preview;
    assert_eq!(db.conn.total_changes(), changes);
    assert_eq!(preview.total_count, 8);
    assert_eq!(preview.protected_count, 2);
    assert_eq!(preview.matched_count, 1);
    assert_eq!(preview.content_bytes, 9);
    assert_eq!(preview.items[0].id, large.id);
    filter.include_grouped = true;
    let grouped = db.preview_cleanup(&filter).unwrap().preview;
    assert_eq!((grouped.matched_count, grouped.content_bytes), (2, 21));
    filter.include_grouped = false;
    filter.kind = "all".into();
    let both = db.preview_cleanup(&filter).unwrap().preview;
    assert_eq!(both.matched_count, 2);
    assert!(both.items.iter().any(|item| item.id == link.id));
    assert_eq!(
        db.preview_cleanup(&all_cleanup())
            .unwrap()
            .preview
            .matched_count,
        5
    );
    filter.kind = "invalid".into();
    assert!(db.preview_cleanup(&filter).is_err());
}

#[test]
fn cleanup_preview_keeps_only_compact_candidates_and_top_100_titles() {
    let mut db = store();
    for index in 0..125 {
        db.add(
            &format!("原创清理样例{index:03}{}", "星".repeat(index)),
            "",
            None,
        )
        .unwrap();
    }
    let snapshot = db.preview_cleanup(&all_cleanup()).unwrap();
    assert_eq!(snapshot.candidates.len(), 125);
    assert!(std::mem::size_of::<CleanupCandidate>() <= 48);
    assert_eq!(snapshot.preview.items.len(), 100);
    assert!(snapshot.preview.truncated);
    assert!(snapshot
        .preview
        .items
        .windows(2)
        .all(|rows| (rows[0].bytes, rows[0].id) >= (rows[1].bytes, rows[1].id)));
    let maximum = db
        .clips()
        .unwrap()
        .into_iter()
        .max_by_key(|clip| (clip_bytes(clip), clip.id))
        .unwrap();
    assert_eq!(snapshot.preview.items[0].id, maximum.id);
    assert_eq!(
        db.execute_cleanup(snapshot, false).unwrap().deleted_count,
        125
    );
}

#[test]
fn stale_cleanup_checks_all_sensitive_fields_and_rolls_back() {
    for change in [
        "content='变更正文'",
        "title='变更备注'",
        "updated_at=updated_at+1",
        "group_id=1",
        "pinned=1",
        "keep_forever=1",
        "kind='code'",
        "language='rust'",
        "created_at=created_at+1",
    ] {
        let mut db = store();
        db.create_group("备选分组", "#ffffff").unwrap();
        db.add("未变化的原创片段", "", None).unwrap();
        let clip = db.add("预览中的片段", "", None).unwrap();
        let snapshot = db.preview_cleanup(&all_cleanup()).unwrap();
        db.conn
            .execute(&format!("UPDATE clips SET {change} WHERE id=?1"), [clip.id])
            .unwrap();
        assert_eq!(
            db.execute_cleanup(snapshot, false).unwrap_err(),
            STALE_CLEANUP
        );
        assert_eq!(
            db.clips().unwrap().len(),
            2,
            "{change} must not partially delete other candidates"
        );
    }
    let mut db = store();
    let old = db.add("被删除的预览项", "", None).unwrap();
    let snapshot = db.preview_cleanup(&all_cleanup()).unwrap();
    db.delete(old.id).unwrap();
    let replacement = db.add("新记录复用了最大编号", "", None).unwrap();
    assert_eq!(replacement.id, old.id);
    assert_eq!(
        db.execute_cleanup(snapshot, false).unwrap_err(),
        STALE_CLEANUP
    );
    assert!(db.clip(replacement.id).is_ok());
}

#[test]
fn cleanup_does_not_delete_new_records_and_failed_delete_is_atomic() {
    let mut db = store();
    let first = db.add("预览片段一", "", None).unwrap();
    db.add("预览片段二", "", None).unwrap();
    let preview = db.preview_cleanup(&all_cleanup()).unwrap();
    let new = db.add("确认之前新收录", "", None).unwrap();
    db.mark_copied(first.id).unwrap(); // copy statistics are not a content edit
    let result = db.execute_cleanup(preview, false).unwrap();
    assert_eq!(result.deleted_count, 2);
    assert_eq!(db.clips().unwrap().len(), 1);
    assert_eq!(db.clips().unwrap()[0].id, new.id);
    db.add("另一条不能丢失的记录", "", None).unwrap();
    let preview = db.preview_cleanup(&all_cleanup()).unwrap();
    db.conn.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON clips WHEN old.content='确认之前新收录' BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
    assert!(db.execute_cleanup(preview, false).is_err());
    assert_eq!(db.clips().unwrap().len(), 2);
}

#[test]
fn compaction_reports_actual_wal_file_reduction_and_handles_empty_match() {
    let scratch = Scratch::new();
    let path = scratch.0.join("cleanup.sqlite3");
    let mut db = Store::open(&path).unwrap();
    db.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    let tx = db.conn.unchecked_transaction().unwrap();
    for index in 0..128 {
        tx.execute(
            "INSERT INTO clips(content,kind,created_at,updated_at) VALUES(?1,'text',0,0)",
            [format!("原创建库 {index} {}", "星".repeat(5500))],
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let before_preview = db.preview_cleanup(&all_cleanup()).unwrap();
    assert_eq!(
        before_preview.preview.database_bytes,
        fs::metadata(&path).unwrap().len()
            + fs::metadata(format!("{}-wal", path.display()))
                .unwrap()
                .len()
    );
    let expected_content_bytes = before_preview.preview.content_bytes;
    let result = db.execute_cleanup(before_preview, false).unwrap();
    assert_eq!(result.deleted_count, 128);
    assert_eq!(result.content_bytes, expected_content_bytes);
    assert_eq!(
        result.released_bytes, 0,
        "deletion alone does not claim free disk space"
    );
    let empty = db.preview_cleanup(&all_cleanup()).unwrap();
    assert_eq!(empty.preview.matched_count, 0);
    assert!(empty.preview.reusable_bytes > 0);
    let result = db.execute_cleanup(empty, true).unwrap();
    assert_eq!(result.deleted_count, 0);
    assert!(result.warning.is_none());
    assert!(result.database_bytes_after < result.database_bytes_before);
    assert_eq!(
        result.released_bytes,
        result.database_bytes_before - result.database_bytes_after
    );
    assert_eq!(result.database_bytes_after, db.database_bytes().unwrap());
    assert!(db.clips().unwrap().is_empty());
}

#[test]
fn checkpoint_failure_returns_committed_deletion_with_warning() {
    let scratch = Scratch::new();
    let path = scratch.0.join("cleanup.sqlite3");
    let mut db = Store::open(&path).unwrap();
    db.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.conn
        .busy_timeout(std::time::Duration::from_millis(20))
        .unwrap();
    db.add("整理失败但删除事务成功", "", None).unwrap();
    let reader = Connection::open(&path).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM clips;")
        .unwrap();
    let preview = db.preview_cleanup(&all_cleanup()).unwrap();
    let result = db.execute_cleanup(preview, true).unwrap();
    assert_eq!(result.deleted_count, 1);
    assert!(result.warning.as_deref().unwrap().contains("整理未完成"));
    assert!(db.clips().unwrap().is_empty());
    assert_eq!(result.database_bytes_after, db.database_bytes().unwrap());
    reader.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn dedup_keeps_metadata_and_group_delete_preserves_content() {
    let db = store();
    let group = db.create_group("工作", "#9977ff").unwrap();
    let first = db.add("你好，星空", "问候", Some(group.id)).unwrap();
    db.update(first.id, "问候", Some(group.id), true).unwrap();
    let again = db.add("你好，星空", "", None).unwrap();
    assert_eq!(first.id, again.id);
    assert_eq!(db.clips().unwrap().len(), 1);
    assert!(again.pinned);
    assert_eq!(again.title, "问候");
    db.delete_group(group.id).unwrap();
    assert_eq!(db.clip(first.id).unwrap().group_id, None);
    db.clear_history().unwrap();
    assert_eq!(db.clips().unwrap().len(), 1);
    db.delete(first.id).unwrap();
    assert!(db.clips().unwrap().is_empty());
}

#[test]
fn retention_preserves_pins_and_groups() {
    let db = store();
    let mut settings = db.settings().unwrap();
    assert!(settings.paused);
    settings.max_history = 50;
    db.save_settings(&settings).unwrap();
    let pinned = db.add("收藏", "", None).unwrap();
    db.update(pinned.id, "", None, true).unwrap();
    let group = db.create_group("常用", "#ffffff").unwrap();
    db.add("分组常用语", "", Some(group.id)).unwrap();
    for i in 0..55 {
        db.add(&format!("第{i}项"), "", None).unwrap();
    }
    assert_eq!(db.clips().unwrap().len(), 52);
    assert!(db.clip(pinned.id).unwrap().pinned);
}

#[test]
fn backup_roundtrip_merge_and_invalid_import_is_atomic() {
    let source = store();
    let group = source.create_group("链接", "#12ab34").unwrap();
    let clip = source
        .add("https://example.com", "主页", Some(group.id))
        .unwrap();
    source
        .update(clip.id, "主页", Some(group.id), true)
        .unwrap();
    let json = source.export().unwrap();
    let mut dest = store();
    assert_eq!(dest.import(&json).unwrap(), 1);
    assert_eq!(dest.import(&json).unwrap(), 0);
    assert_eq!(dest.clips().unwrap()[0].kind, "link");
    assert!(dest.clips().unwrap()[0].pinned);
    assert!(dest.settings().unwrap().paused);
    let mut invalid: serde_json::Value = serde_json::from_str(&json).unwrap();
    invalid["groups"][0]["name"] = "不应导入的分组".into();
    invalid["clips"][0]["content"] = "".into();
    assert!(dest.import(&invalid.to_string()).is_err());
    assert_eq!(dest.groups().unwrap().len(), 1);
    assert_eq!(dest.clips().unwrap().len(), 1);
}

#[test]
fn code_edit_retains_identity_and_conflicts_do_not_overwrite() {
    let mut db = store();
    let code = db
        .save_code(None, "Rust 问候", "fn main() {}", "rust", None)
        .unwrap();
    assert!(code.pinned);
    assert_eq!(code.kind, "code");
    assert_eq!(code.language.as_deref(), Some("rust"));
    db.mark_copied(code.id).unwrap();
    let edited = db
        .save_code(
            Some(code.id),
            "入口",
            "fn main() { println!(\"hi\"); }",
            "rust",
            None,
        )
        .unwrap();
    assert_eq!(edited.id, code.id);
    assert_eq!(edited.copy_count, 1);
    let other = db.add("other", "不能覆盖", None).unwrap();
    assert!(db
        .save_code(Some(code.id), "错误", "other", "rust", None)
        .is_err());
    assert_eq!(db.clip(other.id).unwrap().title, "不能覆盖");
    assert_eq!(db.clip(code.id).unwrap().content, edited.content);
    let backup = db.export().unwrap();
    db.clear_history().unwrap();
    assert_eq!(db.import(&backup).unwrap(), 1);
    assert_eq!(db.clip(code.id).unwrap().language.as_deref(), Some("rust"));
}

#[test]
fn backup_limits_roundtrip_and_reject_overflow() {
    let source = store();
    source.conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000)
        INSERT INTO groups(name,color) SELECT 'group-'||i,'#ffffff' FROM n;
        WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<10000)
        INSERT INTO clips(content,title,kind,pinned,created_at,updated_at) SELECT 'clip-'||i,'','text',1,0,0 FROM n;").unwrap();
    let json = source.export().unwrap();
    let mut destination = store();
    assert_eq!(destination.import(&json).unwrap(), MAX_BACKUP_CLIPS);
    assert_eq!(destination.groups().unwrap().len(), MAX_BACKUP_GROUPS);
    assert!(check_backup_limits(MAX_BACKUP_BYTES, MAX_BACKUP_CLIPS, MAX_BACKUP_GROUPS).is_ok());
    assert!(check_backup_limits(MAX_BACKUP_BYTES + 1, 0, 0).is_err());
    let extra = source.add("overflow", "", None).unwrap();
    assert!(source.export().is_err());
    source.delete(extra.id).unwrap();
    source.create_group("overflow", "#ffffff").unwrap();
    assert!(source.export().is_err());
}

#[test]
fn stale_preference_draft_preserves_newer_capture_choice_atomically() {
    let db = store();
    let mut opened = db.settings().unwrap();
    opened.paused = false;
    db.save_settings(&opened).unwrap();
    opened = db.settings().unwrap();
    let observed_revision = opened.pause_revision;
    let mut draft = opened.clone();
    draft.layout = "list".into();
    draft.always_on_top = true;

    // The tray pauses after the UI read, before its save reaches the store.
    let mut tray = opened;
    tray.paused = true;
    db.save_settings(&tray).unwrap();
    let saved = db
        .save_settings_with_pause_guard(&draft, Some(observed_revision))
        .unwrap();
    assert!(
        saved.paused,
        "an unrelated preference save cannot resume capture"
    );
    assert_eq!(saved.layout, "list");
    assert!(saved.always_on_top);
    assert_eq!(saved.pause_revision, observed_revision + 1);
    assert_eq!(db.settings().unwrap(), saved);

    // A deliberate resume based on the current saved pause still works.
    draft = saved;
    draft.paused = false;
    let resumed = db
        .save_settings_with_pause_guard(&draft, Some(draft.pause_revision))
        .unwrap();
    assert!(!resumed.paused);
    assert_eq!(resumed.pause_revision, draft.pause_revision + 1);
    assert_eq!(db.settings().unwrap(), resumed);

    // Invalid unrelated fields cannot partially commit either preference.
    draft.layout = "invalid".into();
    draft.paused = true;
    assert!(db
        .save_settings_with_pause_guard(&draft, Some(resumed.pause_revision))
        .is_err());
    assert_eq!(db.settings().unwrap(), resumed);
}

#[test]
fn pause_revision_blocks_aba_and_is_owned_by_the_store() {
    let db = store();
    let opened = db.settings().unwrap();
    assert!(opened.paused);
    assert_eq!(opened.pause_revision, 0);
    let mut stale_resume = opened.clone();
    stale_resume.paused = false;
    stale_resume.layout = "list".into();

    // Internal / tray writes must increment even when no guard was supplied.
    let mut tray = opened.clone();
    tray.paused = false;
    tray.pause_revision = u64::MAX;
    db.save_settings(&tray).unwrap();
    assert_eq!(db.settings().unwrap().pause_revision, 1);
    tray.paused = true;
    db.save_settings(&tray).unwrap();
    let paused = db.settings().unwrap();
    assert!(paused.paused);
    assert_eq!(paused.pause_revision, 2);

    // The boolean is back to its old value, but the old resume is stale.
    let saved = db
        .save_settings_with_pause_guard(&stale_resume, Some(opened.pause_revision))
        .unwrap();
    assert!(saved.paused);
    assert_eq!(saved.pause_revision, 2);
    assert_eq!(saved.layout, "list");

    // A current explicit resume works; an arbitrary payload version cannot win.
    let mut fresh_resume = saved.clone();
    fresh_resume.paused = false;
    fresh_resume.pause_revision = u64::MAX;
    let resumed = db
        .save_settings_with_pause_guard(&fresh_resume, Some(saved.pause_revision))
        .unwrap();
    assert!(!resumed.paused);
    assert_eq!(resumed.pause_revision, 3);
    assert_eq!(db.settings().unwrap(), resumed);
    db.save_settings(&fresh_resume).unwrap();
    assert_eq!(
        db.settings().unwrap().pause_revision,
        3,
        "unchanged capture does not advance the version"
    );
    assert_eq!(db.set_theme("forest").unwrap().pause_revision, 3);
}

#[test]
fn legacy_settings_and_backup_keep_their_meaning() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE clips (
        id INTEGER PRIMARY KEY, content TEXT NOT NULL UNIQUE, title TEXT NOT NULL DEFAULT '',
        kind TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0, group_id INTEGER,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, copy_count INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT INTO clips(content,kind,pinned,created_at,updated_at) VALUES('旧版原创片段','text',1,0,0);").unwrap();
    let legacy = r#"{"paused":true,"theme":"light","layout":"list","maxHistory":500,"shortcut":"CommandOrControl+Shift+V"}"#;
    conn.execute(
        "INSERT INTO preferences(key,value) VALUES('settings',?1)",
        [legacy],
    )
    .unwrap();
    let db = Store::from_connection(conn).unwrap();
    let settings = db.settings().unwrap();
    assert_eq!(
        settings.pause_revision, 0,
        "old stored settings need no migration write"
    );
    assert!(!settings.always_on_top);
    assert!(settings.show_startup);
    assert_eq!(settings.sticker_pack, "auto");
    assert_eq!(
        settings.max_history, 500,
        "legacy configured limits remain unchanged"
    );
    assert_eq!(settings.retention_days, 0);
    assert_eq!(settings.layout, "list");
    assert_eq!(db.clips().unwrap()[0].language, None);
    assert!(!db.clips().unwrap()[0].keep_forever);
    assert_eq!(
        db.conn
            .query_row(
                "SELECT value FROM preferences WHERE key='settings'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        legacy
    );
    let mut old_backup: serde_json::Value = serde_json::from_str(&db.export().unwrap()).unwrap();
    old_backup["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("language");
    old_backup["clips"][0]
        .as_object_mut()
        .unwrap()
        .remove("keepForever");
    let mut destination = store();
    let mut updated = settings.clone();
    updated.layout = "horizontal".into();
    updated.always_on_top = true;
    updated.retention_days = 7;
    updated.show_startup = false;
    updated.sticker_pack = "cats".into();
    for theme in ["forest", "rose", "white"] {
        updated.theme = theme.into();
        destination.save_settings(&updated).unwrap();
        assert_eq!(destination.settings().unwrap(), updated);
    }
    destination.save_settings(&updated).unwrap();
    assert_eq!(destination.import(&old_backup.to_string()).unwrap(), 1);
    assert_eq!(destination.settings().unwrap(), updated);
    assert!(destination.clips().unwrap()[0].pinned);
    assert!(!destination.clips().unwrap()[0].keep_forever);
    updated.retention_days = 2;
    assert!(destination.save_settings(&updated).is_err());
    assert_eq!(destination.settings().unwrap().retention_days, 7);
}

#[test]
fn age_cleanup_preserves_protected_items_and_recapture_renews_history() {
    let db = store();
    let mut settings = db.settings().unwrap();
    settings.retention_days = 7;
    db.save_settings(&settings).unwrap();
    let expired = db.add("过期片段", "", None).unwrap();
    let renewed = db.add("再次收录", "", None).unwrap();
    let pinned = db.add("重要星标", "", None).unwrap();
    db.update(pinned.id, "", None, true).unwrap();
    let group = db.create_group("常用片段", "#aabbcc").unwrap();
    let grouped = db.add("分组保护", "", Some(group.id)).unwrap();
    db.conn
        .execute("UPDATE clips SET created_at=0,updated_at=0", [])
        .unwrap();
    let fresh = db.add("再次收录", "", None).unwrap();
    assert_eq!(fresh.id, renewed.id);
    assert_eq!(fresh.created_at, 0);
    assert!(fresh.updated_at > 0);
    assert!(db.clip(expired.id).is_err());
    assert!(db.clip(pinned.id).is_ok());
    assert!(db.clip(grouped.id).is_ok());
    let boundary = fresh.updated_at - 7 * 86_400_000;
    db.conn
        .execute(
            "UPDATE clips SET updated_at=?1 WHERE id=?2",
            params![boundary, fresh.id],
        )
        .unwrap();
    assert_eq!(
        Store::prune_with_settings(&db.conn, &settings, fresh.updated_at).unwrap(),
        0
    );
    assert_eq!(
        Store::prune_with_settings(&db.conn, &settings, fresh.updated_at + 1).unwrap(),
        1
    );
    assert_eq!(db.clips().unwrap().len(), 2);
}

#[test]
fn age_and_capacity_apply_together_at_startup_and_settings_are_atomic() {
    let db = store();
    let mut settings = db.settings().unwrap();
    settings.retention_days = 7;
    settings.max_history = 50;
    db.save_settings(&settings).unwrap();
    let timestamp = now();
    // Seed history directly so one maintenance pass exercises both limits together.
    let tx = db.conn.unchecked_transaction().unwrap();
    for index in 0..60 {
        let updated = if index < 5 { 0 } else { timestamp + index };
        tx.execute(
            "INSERT INTO clips(content,kind,created_at,updated_at) VALUES(?1,'text',0,?2)",
            params![format!("容量与时间测试 {index}"), updated],
        )
        .unwrap();
    }
    tx.execute("INSERT INTO clips(content,kind,pinned,created_at,updated_at) VALUES('受保护星标','text',1,0,0)", []).unwrap();
    tx.commit().unwrap();
    let reopened = Store::from_connection(db.conn).unwrap();
    assert_eq!(reopened.clips().unwrap().len(), 51);
    assert_eq!(reopened.prune_history().unwrap(), 0);
    assert!(reopened
        .clips()
        .unwrap()
        .iter()
        .any(|clip| clip.content == "容量与时间测试 10"));
    assert!(!reopened
        .clips()
        .unwrap()
        .iter()
        .any(|clip| clip.content == "容量与时间测试 9"));
    reopened
        .conn
        .execute("UPDATE clips SET updated_at=0 WHERE pinned=0", [])
        .unwrap();
    reopened.conn.execute_batch("CREATE TRIGGER reject_cleanup BEFORE DELETE ON clips BEGIN SELECT RAISE(ABORT, 'test cleanup failure'); END;").unwrap();
    let mut changed = settings.clone();
    changed.retention_days = 1;
    changed.always_on_top = true;
    assert!(reopened.save_settings(&changed).is_err());
    assert_eq!(reopened.settings().unwrap(), settings);
    assert_eq!(reopened.clips().unwrap().len(), 51);
}

#[test]
fn image_migration_preserves_legacy_fields_and_separates_text_identity() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;
        CREATE TABLE groups(id INTEGER PRIMARY KEY,name TEXT NOT NULL UNIQUE,color TEXT NOT NULL);
        INSERT INTO groups VALUES(7,'原创旧分组','#abcdef');
        CREATE TABLE clips(id INTEGER PRIMARY KEY,content TEXT NOT NULL UNIQUE,title TEXT NOT NULL DEFAULT '',
            kind TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,keep_forever INTEGER NOT NULL DEFAULT 0,
            group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,copy_count INTEGER NOT NULL DEFAULT 0,language TEXT);
        INSERT INTO clips VALUES(11,'fn original() {}','旧代码','code',1,1,7,12,34,9,'rust');").unwrap();
    let db = Store::from_connection(conn).unwrap();
    let legacy = db.clip(11).unwrap();
    assert_eq!(
        (legacy.created_at, legacy.updated_at, legacy.copy_count),
        (12, 34, 9)
    );
    assert_eq!(legacy.group_id, Some(7));
    assert!(legacy.pinned && legacy.keep_forever);
    assert_eq!(legacy.language.as_deref(), Some("rust"));
    assert_eq!(legacy.title, "旧代码");
    assert!(legacy.image.is_none());
    let old_backup: serde_json::Value = serde_json::from_str(&db.export().unwrap()).unwrap();
    assert_eq!(old_backup["version"], 3);
    assert!(old_backup.get("images").is_none());

    let mut budget_clip = legacy.clone();
    budget_clip.image = Some(ImageMetadata {
        width: 4000,
        height: 4000,
        bytes: 100,
    });
    assert!(check_backup_image_limits(&vec![budget_clip.clone(); 16]).is_ok());
    assert!(check_backup_image_limits(&vec![budget_clip; 17]).is_err());

    let pixels = image_fixture(50);
    let text = db.add(pixels.digest(), "恰好相同的普通文本", None).unwrap();
    let picture = db.add_image(&pixels, "图片", Some(7), true).unwrap();
    assert_ne!(text.id, picture.id);
    db.update(picture.id, "图片标题", Some(7), true).unwrap();
    db.mark_copied(picture.id).unwrap();
    db.conn
        .execute("UPDATE clips SET updated_at=0 WHERE id=?1", [picture.id])
        .unwrap();
    let repeated = db.add_image(&pixels, "", None, false).unwrap();
    assert_eq!(picture.id, repeated.id);
    assert!(repeated.updated_at > 0);
    assert!(repeated.pinned && repeated.keep_forever);
    assert_eq!(repeated.copy_count, 1);
    assert_eq!(repeated.group_id, Some(7));
    assert_eq!(repeated.title, "图片标题");
    assert_eq!(db.add(pixels.digest(), "", None).unwrap().id, text.id);
    assert_eq!(db.clip(text.id).unwrap().kind, "text");
    assert!(db
        .save_code(Some(picture.id), "不应转换", "hello", "plaintext", None)
        .is_err());
    let list_json = serde_json::to_value(db.clips().unwrap()).unwrap();
    let listed = list_json
        .as_array()
        .unwrap()
        .iter()
        .find(|clip| clip["id"] == picture.id)
        .unwrap();
    assert_eq!(listed["image"]["bytes"], pixels.png.len());
    assert!(listed.get("png").is_none());
    db.delete_group(7).unwrap();
    assert!(db.clip(11).unwrap().group_id.is_none());
    assert!(db.clip(picture.id).unwrap().group_id.is_none());
    let reopened = Store::from_connection(db.conn).unwrap();
    assert_eq!(reopened.image_png(picture.id, false).unwrap(), pixels.png);
}

#[test]
fn image_backup_roundtrip_validation_and_failure_are_atomic() {
    let source = store();
    let group = source.create_group("原创像素", "#abcdef").unwrap();
    let pixels = image_fixture(91);
    let picture = source
        .add_image(&pixels, "透明像素测试", Some(group.id), true)
        .unwrap();
    source
        .update(picture.id, "透明像素测试", Some(group.id), true)
        .unwrap();
    source.add(pixels.digest(), "相同标识的文本", None).unwrap();
    let json = source.export().unwrap();
    let backup: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(backup["version"], 3);
    assert_eq!(
        STANDARD
            .decode(backup["images"][0]["png"].as_str().unwrap())
            .unwrap(),
        pixels.png
    );
    let mut destination = store();
    assert_eq!(destination.import(&json).unwrap(), 2);
    assert_eq!(destination.import(&json).unwrap(), 0);
    let imported = destination
        .clips()
        .unwrap()
        .into_iter()
        .find(|clip| clip.kind == "image")
        .unwrap();
    assert_eq!(
        destination.image_png(imported.id, false).unwrap(),
        pixels.png
    );
    assert_eq!(
        destination.image_png(imported.id, true).unwrap(),
        pixels.thumbnail
    );
    assert!(imported.pinned && imported.keep_forever && imported.group_id.is_some());
    assert!(destination.settings().unwrap().paused);
    let mut unprotected = backup.clone();
    for clip in unprotected["clips"].as_array_mut().unwrap() {
        clip["keepForever"] = false.into();
        clip["pinned"] = false.into();
    }
    destination.import(&unprotected.to_string()).unwrap();
    assert!(destination.clip(imported.id).unwrap().keep_forever);
    assert!(destination.clip(imported.id).unwrap().pinned);

    let picture_index = backup["clips"]
        .as_array()
        .unwrap()
        .iter()
        .position(|clip| clip["kind"] == "image")
        .unwrap();
    for fault in 0..9 {
        let mut broken = backup.clone();
        broken["groups"][0]["name"] = "不应留下的分组".into();
        match fault {
            0 => broken["images"] = serde_json::json!([]),
            1 => broken["images"][0]["png"] = "not-base64".into(),
            2 => broken["clips"][picture_index]["image"]["width"] = 99.into(),
            3 => broken["clips"][picture_index]["image"]["bytes"] = 1.into(),
            4 => broken["clips"][picture_index]["content"] = "sha256:forged".into(),
            5 => broken["version"] = 1.into(),
            6 => broken["images"][0]["clipId"] = 9999.into(),
            7 => {
                let duplicate = broken["images"][0].clone();
                broken["images"].as_array_mut().unwrap().push(duplicate);
            }
            _ => {
                let mut png = pixels.png.clone();
                png[29] ^= 1;
                broken["images"][0]["png"] = STANDARD.encode(png).into();
            }
        }
        assert!(
            destination.import(&broken.to_string()).is_err(),
            "fault {fault}"
        );
        assert_eq!(destination.clips().unwrap().len(), 2);
        assert_eq!(destination.groups().unwrap().len(), 1);
    }
    let mut failing = store();
    failing.conn.execute_batch("CREATE TRIGGER refuse_image BEFORE INSERT ON clip_images BEGIN SELECT RAISE(ABORT, 'fixture image write failure'); END;").unwrap();
    assert!(failing.import(&json).is_err());
    assert!(failing.clips().unwrap().is_empty());
    assert!(failing.groups().unwrap().is_empty());
    assert!(failing.add_image(&pixels, "写入失败", None, false).is_err());
    assert!(failing.clips().unwrap().is_empty());
}

#[test]
fn image_cleanup_bytes_protection_and_cascades_cover_all_delete_paths() {
    let mut db = store();
    let pixels = image_fixture(12);
    let picture = db.add_image(&pixels, "星图", None, false).unwrap();
    let expected = pixels.png.len() as u64 + "星图".len() as u64;
    let mut filter = all_cleanup();
    filter.kind = "image".into();
    filter.min_bytes = expected;
    assert_eq!(
        db.preview_cleanup(&filter).unwrap().preview.matched_count,
        0
    );
    filter.min_bytes -= 1;
    let preview = db.preview_cleanup(&filter).unwrap();
    assert_eq!(preview.preview.content_bytes, expected);
    assert_eq!(preview.preview.items[0].bytes, expected);
    db.set_keep_forever(picture.id, true).unwrap();
    assert!(db.execute_cleanup(preview, false).is_err());
    assert!(db.delete(picture.id).is_err());
    db.clear_history().unwrap();
    assert!(db.image_png(picture.id, false).is_ok());
    db.set_keep_forever(picture.id, false).unwrap();
    let preview = db.preview_cleanup(&filter).unwrap();
    let next = db
        .add_image(&image_fixture(13), "预览之后新图片", None, false)
        .unwrap();
    let result = db.execute_cleanup(preview, false).unwrap();
    assert_eq!(result.deleted_count, 1);
    assert_eq!(result.content_bytes, expected);
    assert!(db.image_png(picture.id, false).is_err());
    assert!(db.image_png(next.id, false).is_ok());
    db.delete(next.id).unwrap();
    assert_eq!(
        db.conn
            .query_row("SELECT count(*) FROM clip_images", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let group = db.create_group("保护图片", "#123abc").unwrap();
    let protected = db
        .add_image(&image_fixture(20), "永久", None, true)
        .unwrap();
    let pinned = db
        .add_image(&image_fixture(21), "星标", None, false)
        .unwrap();
    db.update(pinned.id, "星标", None, true).unwrap();
    let grouped = db
        .add_image(&image_fixture(22), "分组", Some(group.id), false)
        .unwrap();
    let expired = db
        .add_image(&image_fixture(23), "超时", None, false)
        .unwrap();
    db.conn
        .execute("UPDATE clips SET updated_at=0", [])
        .unwrap();
    let settings = Settings {
        retention_days: 1,
        ..Settings::default()
    };
    db.save_settings(&settings).unwrap();
    assert!(db.image_png(expired.id, false).is_err());
    for id in [protected.id, pinned.id, grouped.id] {
        assert!(db.image_png(id, false).is_ok());
    }
    db.clear_history().unwrap();
    assert!(db.image_png(grouped.id, false).is_err());
    for id in [protected.id, pinned.id] {
        assert!(db.image_png(id, false).is_ok());
    }
    let mut settings = settings;
    settings.retention_days = 0;
    settings.max_history = 50;
    db.save_settings(&settings).unwrap();
    let old = db
        .add_image(&image_fixture(24), "容量淘汰", None, false)
        .unwrap();
    for index in 0..50 {
        db.add(&format!("原创容量片段 {index}"), "", None).unwrap();
    }
    assert!(db.image_png(old.id, false).is_err());
    assert_eq!(
        db.conn
            .query_row("SELECT count(*) FROM clip_images", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn wal_snapshot_preserves_original_and_thumbnail_without_external_assets() {
    let scratch = Scratch::new();
    let path = scratch.0.join("images.sqlite3");
    let snapshot = scratch.0.join("snapshot.sqlite3");
    let db = Store::open(&path).unwrap();
    db.conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    db.conn
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    let pixels = images::prepare_rgba(640, 160, &[90, 120, 210, 120].repeat(640 * 160)).unwrap();
    let clip = db
        .add_image(&pixels, "原创半透明横幅", None, false)
        .unwrap();
    db.snapshot(&snapshot).unwrap();
    Store::validate_database(&snapshot).unwrap();
    db.delete(clip.id).unwrap();
    let copy = Store::open(&snapshot).unwrap();
    assert_eq!(copy.image_png(clip.id, false).unwrap(), pixels.png);
    assert_eq!(copy.image_png(clip.id, true).unwrap(), pixels.thumbnail);
    assert_eq!(
        copy.clip(clip.id).unwrap().image.as_ref(),
        Some(pixels.metadata())
    );
    assert_eq!(
        images::decode_png(&copy.image_png(clip.id, true).unwrap())
            .unwrap()
            .0,
        320
    );
    assert!(db.clips().unwrap().is_empty());
}

#[test]
fn unicode_and_limits_are_checked_without_mutation() {
    let db = store();
    assert!(db.add("  ", "", None).is_err());
    assert!(db
        .add(&"x".repeat(MAX_CONTENT_BYTES + 1), "", None)
        .is_err());
    assert!(db.create_group("星", "#星星").is_err());
    assert_eq!(classify("#a1b2c3"), "color");
    assert_eq!(classify("https://example.com\nhello"), "text");
    assert!(db.clips().unwrap().is_empty());
}
#[test]
fn ui_language_defaults_to_system_persists_and_survives_theme_writes() {
    let scratch = Scratch::new();
    let path = scratch.0.join("language.sqlite3");
    let db = Store::open(&path).unwrap();
    assert_eq!(db.settings().unwrap().ui_language, "system");

    // A preference written before the language field existed keeps working.
    let legacy = r#"{"paused":true,"theme":"dark","layout":"grid","maxHistory":0,"shortcut":"CommandOrControl+Shift+V"}"#;
    db.conn
        .execute(
            "INSERT INTO preferences(key,value) VALUES('settings',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [legacy],
        )
        .unwrap();
    assert_eq!(db.settings().unwrap().ui_language, "system");

    let mut settings = db.settings().unwrap();
    settings.ui_language = "en-US".into();
    db.save_settings(&settings).unwrap();
    assert_eq!(db.settings().unwrap().ui_language, "en-US");

    // The immediate theme write must preserve other stored preferences.
    db.set_theme("forest").unwrap();
    let saved = db.settings().unwrap();
    assert_eq!(saved.ui_language, "en-US");
    assert_eq!(saved.theme, "forest");

    let unsupported = Settings {
        ui_language: "fr".into(),
        ..Settings::default()
    };
    assert!(unsupported.validate().is_err());
}

#[test]
fn rich_text_and_file_captures_keep_formats_separately() {
    let db = store();
    let ctx = CaptureContext::default();
    let rich = db
        .capture_rich_text("原创加粗样例", Some("<b>原创加粗样例</b>"), &ctx)
        .unwrap();
    assert!(rich.rich);
    assert_eq!(
        db.clip_html(rich.id).unwrap().as_deref(),
        Some("<b>原创加粗样例</b>")
    );
    // The latest copy decides: copying the same text without formatting drops the HTML.
    let plain = db.capture_text("原创加粗样例", &ctx).unwrap();
    assert!(!plain.rich);
    assert_eq!(db.clip_html(rich.id).unwrap(), None);
    let huge = "x".repeat(MAX_RICH_BYTES + 1);
    assert!(
        !db.capture_rich_text("原创超长格式", Some(&huge), &ctx)
            .unwrap()
            .rich
    );

    let files = db
        .capture_files(
            &["/fixture/原创/a.txt".into(), "/fixture/原创/b.png".into()],
            &ctx,
        )
        .unwrap();
    assert_eq!(files.kind, "file");
    assert_eq!(files.content, "/fixture/原创/a.txt\n/fixture/原创/b.png");
    assert!(db.capture_files(&[], &ctx).is_err());
    assert!(db.capture_files(&["bad\npath".into()], &ctx).is_err());
    assert!(db
        .save_code(Some(files.id), "", "x", "plaintext", None)
        .is_err());
    let page = db
        .history_page(&HistoryQuery {
            filter: "file".into(),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(
        page.clips.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![files.id]
    );
    db.delete(rich.id).unwrap();
    let orphans: i64 = db
        .conn
        .query_row("SELECT count(*) FROM clip_rich", [], |row| row.get(0))
        .unwrap();
    assert_eq!(orphans, 0);
}

#[test]
fn trigram_index_matches_scan_results_and_forgets_deleted_text() {
    let db = store();
    let a = db.add("原创星图样例 ÄBC fixture", "", None).unwrap();
    let b = db.add("另一段原创星图", "原创标题", None).unwrap();
    let c = db.add("无关内容", "", None).unwrap();
    let ids = |query: &str| {
        let mut ids = db
            .history_page(&HistoryQuery {
                query: query.into(),
                ..HistoryQuery::default()
            })
            .unwrap()
            .clips
            .iter()
            .map(|clip| clip.id)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    };
    assert_eq!(ids("原创星图"), vec![a.id, b.id]);
    assert_eq!(ids("äbc"), vec![a.id], "Unicode lowercase still matches");
    assert_eq!(
        ids("星图"),
        vec![a.id, b.id],
        "short queries fall back to scanning"
    );
    assert_eq!(ids("原创标题"), vec![b.id], "titles are indexed");
    db.update(c.id, "新的原创星图标题", None, false).unwrap();
    assert_eq!(
        ids("原创星图"),
        vec![a.id, b.id, c.id],
        "updates refresh the index"
    );
    db.delete(a.id).unwrap();
    assert_eq!(ids("原创星图"), vec![b.id, c.id]);
    let indexed: i64 = db
        .conn
        .query_row(
            "SELECT count(*) FROM clips_fts WHERE clips_fts MATCH '\"ÄBC fixture\"'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(indexed, 0, "deleted text leaves no index entry");
    // A legacy rebuild of the clips table drops the triggers; reopening rebuilds the index.
    db.conn
        .execute_batch("DROP TRIGGER clips_fts_insert; DELETE FROM clips_fts;")
        .unwrap();
    crate::search_index::ensure(&db.conn).unwrap();
    assert_eq!(ids("原创星图"), vec![b.id, c.id]);
}

#[test]
fn capture_context_marks_privacy_app_tags_and_origin_without_query() {
    let db = store();
    let concealed = CaptureContext {
        concealed: true,
        app: Some("  原创密码本\n".into()),
        origin_url: None,
    };
    let clip = db.capture_text("原创样例口令片段", &concealed).unwrap();
    assert!(clip.is_private);
    assert!(clip
        .privacy_reasons
        .contains(&privacy::CONCEALED_REASON.to_string()));
    assert_eq!(clip.app_tags, vec!["原创密码本"]);
    assert!(clip.tags.contains(&"原创密码本".to_string()));
    // A later ordinary copy of the same text keeps the protection and adds the app.
    let again = db
        .capture_text(
            "原创样例口令片段",
            &CaptureContext {
                app: Some("原创编辑器".into()),
                ..CaptureContext::default()
            },
        )
        .unwrap();
    assert!(again.is_private);
    assert_eq!(again.app_tags, vec!["原创密码本", "原创编辑器"]);

    let browser = CaptureContext {
        app: Some("原创浏览器".into()),
        origin_url: Some("https://user:pw@linux.do/t/fixture?token=abc#frag".into()),
        ..CaptureContext::default()
    };
    let page = db.capture_text("原创网页摘录", &browser).unwrap();
    assert_eq!(
        page.origin_url.as_deref(),
        Some("https://linux.do/t/fixture")
    );
    assert!(!page.is_private);
    assert!(
        page.link_hosts.is_empty(),
        "origin is not a link in the content"
    );
    let query = |sources: Vec<String>, tag: &str| HistoryQuery {
        sources,
        tag: tag.into(),
        ..HistoryQuery::default()
    };
    let by_host = db
        .history_page(&query(vec!["host:linux.do".into()], ""))
        .unwrap();
    assert_eq!(
        by_host.clips.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![page.id]
    );
    let by_source = db.history_page(&query(vec!["linuxdo".into()], "")).unwrap();
    assert_eq!(by_source.clips.len(), 1);
    let by_app = db.history_page(&query(vec![], "原创编辑器")).unwrap();
    assert_eq!(
        by_app.clips.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![clip.id]
    );
    assert!(by_app.history.tags.contains(&"原创浏览器".to_string()));
    assert!(by_app.history.hosts.contains(&"linux.do".to_string()));

    // Domain rules also match the browser origin.
    let mut rules = db.privacy_rules().unwrap();
    rules.rules.push(privacy::PrivacyRule {
        id: "origin-rule".into(),
        name: "原创来源规则".into(),
        enabled: true,
        condition: "domain".into(),
        value: "linux.do".into(),
        tag: None,
        mark_private: true,
    });
    let other = db.capture_text("原创另一段摘录", &browser).unwrap();
    assert!(!other.is_private);
    db.save_privacy_rules(&rules).unwrap();
    let rescanned = db
        .capture_text("原创另一段摘录", &CaptureContext::default())
        .unwrap();
    assert!(rescanned.is_private, "{:?}", rescanned.privacy_reasons);
}

#[test]
fn clip_stats_count_captures_repeats_and_reuse_without_content() {
    let db = store();
    let first = db
        .capture_text("原创星迹样例", &CaptureContext::default())
        .unwrap();
    db.capture_text("原创星迹样例", &CaptureContext::default())
        .unwrap();
    db.capture_text(
        "https://linux.do/t/stats-fixture",
        &CaptureContext::default(),
    )
    .unwrap();
    db.capture_image(&image_fixture(31), &CaptureContext::default())
        .unwrap();
    db.add("手动新建不算采集", "", None).unwrap();
    db.mark_copied(first.id).unwrap();
    let stats = db.clip_stats(7, 0).unwrap();
    assert_eq!(stats.daily.len(), 1);
    let today = &stats.daily[0];
    assert_eq!((today.new, today.repeat, today.reuse), (3, 1, 1));
    assert_eq!(today.captures.get("text"), Some(&2));
    assert_eq!(today.captures.get("link"), Some(&1));
    assert_eq!(today.captures.get("image"), Some(&1));
    assert_eq!(
        stats.kinds["text"].size, 12,
        "two captures of six characters"
    );
    assert!(stats.kinds["image"].size > 0);
    assert_eq!(stats.hours.iter().sum::<u64>(), 4);
    assert_eq!(stats.weekdays.iter().sum::<u64>(), 4);
    assert_eq!(stats.library.total, 4);
    assert_eq!(stats.library.copy_total, 1);
    assert_eq!(stats.library.top_hosts.len(), 1);
    let columns: Vec<String> = db
        .conn
        .prepare("SELECT name FROM pragma_table_info('clip_activity')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(columns, ["bucket", "kind", "event", "count", "size"]);

    db.clear_history().unwrap();
    assert_eq!(
        db.clip_stats(7, 0).unwrap().daily[0].new,
        3,
        "cleanup keeps counts"
    );
    db.clear_clip_stats().unwrap();
    let cleared = db.clip_stats(7, 0).unwrap();
    assert!(cleared.daily.is_empty() && cleared.recording_since.is_none());
}

#[test]
fn clip_stats_repeat_keeps_saved_kind_and_hide_private_hosts() {
    let db = store();
    let clip = db
        .capture_text("let fixture = 1", &CaptureContext::default())
        .unwrap();
    db.save_code(Some(clip.id), "", "let fixture = 1", "rust", None)
        .unwrap();
    db.capture_text("let fixture = 1", &CaptureContext::default())
        .unwrap();
    let private = db
        .capture_text(
            "https://private.example/fixture",
            &CaptureContext::default(),
        )
        .unwrap();
    db.set_clip_privacy(private.id, true, false).unwrap();
    let stats = db.clip_stats(1, 0).unwrap();
    assert_eq!(stats.kinds["code"].repeat, 1);
    assert_eq!(stats.library.private, 1);
    assert!(stats.library.top_hosts.is_empty());
}

#[test]
fn clip_stats_group_by_local_day_and_prune_old_buckets() {
    let db = store();
    // 2026-01-01T16:30Z is 00:30 on 2026-01-02 in UTC+8, a Friday.
    let at = 1_767_285_000_000;
    clip_stats::record(&db.conn, at, "text", clip_stats::Event::New, 5).unwrap();
    clip_stats::record(&db.conn, at, "unknown", clip_stats::Event::New, 1).unwrap();
    let stats = clip_stats::activity(&db.conn, at, 1, 480, PRIVATE_PREDICATE).unwrap();
    assert_eq!(stats.daily.len(), 1);
    assert_eq!(stats.daily[0].day, stats.last_day);
    assert_eq!(
        stats.daily[0].captures.get("text"),
        Some(&2),
        "unknown kinds fold into text"
    );
    assert_eq!(stats.hours[0], 2);
    assert_eq!(stats.weekdays[5], 2);
    let utc = clip_stats::activity(&db.conn, at, 1, 0, PRIVATE_PREDICATE).unwrap();
    assert_eq!(utc.hours[16], 2);
    let tomorrow =
        clip_stats::activity(&db.conn, at + 86_400_000, 1, 480, PRIVATE_PREDICATE).unwrap();
    assert!(
        tomorrow.daily.is_empty(),
        "range ends on the local day of now"
    );
    let later = at + (clip_stats::RETENTION_DAYS + 1) * 86_400_000;
    clip_stats::record(&db.conn, later, "link", clip_stats::Event::Reuse, 0).unwrap();
    let since = clip_stats::activity(&db.conn, later, 366, 0, PRIVATE_PREDICATE).unwrap();
    assert_eq!(
        since.recording_since,
        Some(later.div_euclid(clip_stats::BUCKET_MS) * clip_stats::BUCKET_MS)
    );
    assert!(clip_stats::validate_range(0, 0).is_err());
    assert!(clip_stats::validate_range(367, 0).is_err());
    assert!(clip_stats::validate_range(7, 7).is_err());
    assert!(clip_stats::validate_range(7, 345).is_ok());
}

#[test]
fn current_schema_gains_activity_table_without_version_change() {
    let db = store();
    db.conn.execute("DROP TABLE clip_activity", []).unwrap();
    let db = Store::from_connection(db.conn).unwrap();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(db.clip_stats(7, 0).unwrap().daily.is_empty());
    db.capture_text("原创升级后计数", &CaptureContext::default())
        .unwrap();
    assert_eq!(db.clip_stats(7, 0).unwrap().daily[0].new, 1);
}
