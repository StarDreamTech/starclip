use super::*;

fn memory_store() -> Store {
    Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
}

fn page(store: &Store) -> HistoryPage {
    store.history_page(&HistoryQuery::default()).unwrap()
}

fn rebuilt_page(store: &Store) -> HistoryPage {
    let before = store.history_global_query_count();
    let result = page(store);
    assert_eq!(store.history_global_query_count(), before + 1);
    result
}

fn global_fields(page: &HistoryPage) -> serde_json::Value {
    serde_json::json!({
        "counts": page.history.counts,
        "groupCounts": page.history.group_counts,
        "sources": page.history.sources,
        "hosts": page.history.hosts,
        "tags": page.history.tags,
    })
}

#[test]
fn history_cache_reuses_global_facets_and_tracks_committed_library_operations() {
    let mut store = memory_store();
    assert_eq!(rebuilt_page(&store).history.counts["all"], 0);
    let group = store.create_group("原创缓存分组", "#778899").unwrap();
    let link = store
        .capture_text(
            "https://linux.do/original-cache-orbit",
            &CaptureContext {
                app: Some("原创缓存编辑器".into()),
                origin_url: Some("https://example.test/original-cache-source".into()),
                ..CaptureContext::default()
            },
        )
        .unwrap();
    store
        .update(link.id, "唯一定轨样例", Some(group.id), true)
        .unwrap();
    store.set_keep_forever(link.id, true).unwrap();
    store
        .set_clip_tags(link.id, &["旧缓存标签".into()])
        .unwrap();
    for number in 0..60 {
        store
            .add(&format!("原创分页缓存内容 {number}"), "原创分页记录", None)
            .unwrap();
    }
    let warm = rebuilt_page(&store);
    assert_eq!(warm.history.total, 61);
    assert_eq!(warm.history.counts["pinned"], 1);
    assert_eq!(warm.history.counts["kept"], 1);
    assert_eq!(warm.history.group_counts[&group.id], 1);
    assert!(warm.history.hosts.contains(&"linux.do".into()));
    assert!(warm.history.hosts.contains(&"example.test".into()));
    assert!(warm.history.tags.contains(&"旧缓存标签".into()));
    let queries = store.history_global_query_count();
    for query in [
        HistoryQuery {
            page: 2,
            ..HistoryQuery::default()
        },
        HistoryQuery {
            query: "唯一定轨样例".into(),
            ..HistoryQuery::default()
        },
        HistoryQuery {
            filter: "pinned".into(),
            ..HistoryQuery::default()
        },
        HistoryQuery {
            tag: "旧缓存标签".into(),
            sources: vec!["host:linux.do".into()],
            ..HistoryQuery::default()
        },
    ] {
        assert_eq!(
            global_fields(&store.history_page(&query).unwrap()),
            global_fields(&warm)
        );
    }
    for filter in [
        "all",
        "pinned",
        "kept",
        "link",
        "text",
        "code",
        "image",
        "file",
        "color",
        &format!("group-{}", group.id),
    ] {
        let predicate = match filter {
            "all" => "1=1".to_owned(),
            "pinned" => "c.pinned=1".into(),
            "kept" => "c.keep_forever=1".into(),
            "link" => "c.kind='link' OR json_array_length(p.metadata,'$.linkHosts')>0".into(),
            filter if filter.starts_with("group-") => format!("c.group_id={}", group.id),
            kind => format!("c.kind='{kind}'"),
        };
        let actual: u64 = store.conn.query_row(&format!("SELECT COUNT(*) FROM clips c LEFT JOIN clip_privacy p ON p.clip_id=c.id WHERE {predicate}"), [], |row| row.get(0)).unwrap();
        assert_eq!(
            store
                .history_page(&HistoryQuery {
                    filter: filter.into(),
                    ..HistoryQuery::default()
                })
                .unwrap()
                .history
                .total,
            actual,
            "unfiltered {filter} total still equals live SQL"
        );
    }
    assert_eq!(
        store.history_global_query_count(),
        queries,
        "searches and pages reuse the same global summary"
    );

    store
        .set_clip_tags(link.id, &["新缓存标签".into()])
        .unwrap();
    let retagged = rebuilt_page(&store);
    assert!(retagged.history.tags.contains(&"新缓存标签".into()));
    assert!(!retagged.history.tags.contains(&"旧缓存标签".into()));
    store.update(link.id, "唯一定轨样例", None, false).unwrap();
    store.set_keep_forever(link.id, false).unwrap();
    store.delete_group(group.id).unwrap();
    let unprotected = rebuilt_page(&store);
    assert_eq!(unprotected.history.counts["pinned"], 0);
    assert_eq!(unprotected.history.counts["kept"], 0);
    assert!(unprotected.history.group_counts.is_empty());
    store.delete(link.id).unwrap();
    let removed = rebuilt_page(&store);
    assert_eq!(removed.history.counts["all"], 60);
    assert!(removed.history.tags.is_empty());
    assert!(removed.history.hosts.is_empty());

    let donor = memory_store();
    let imported = donor
        .add(
            "https://example.test/original-cache-import",
            "原创导入样例",
            None,
        )
        .unwrap();
    donor
        .set_clip_tags(imported.id, &["导入缓存标签".into()])
        .unwrap();
    assert_eq!(store.import(&donor.export().unwrap()).unwrap(), 1);
    let merged = rebuilt_page(&store);
    assert_eq!(merged.history.counts["all"], 61);
    assert!(merged.history.tags.contains(&"导入缓存标签".into()));
    assert!(merged.history.hosts.contains(&"example.test".into()));
    let snapshot = store
        .preview_cleanup(&CleanupFilter {
            min_bytes: 0,
            older_than_days: 0,
            kind: "all".into(),
            include_grouped: true,
        })
        .unwrap();
    assert_eq!(
        store
            .execute_cleanup(snapshot, false)
            .unwrap()
            .deleted_count,
        61
    );
    let empty = rebuilt_page(&store);
    assert_eq!(empty.history.counts["all"], 0);
    assert!(
        empty.history.tags.is_empty()
            && empty.history.hosts.is_empty()
            && empty.history.sources.is_empty()
    );
}

#[test]
fn history_cache_never_caches_record_privacy_or_ai_eligibility() {
    let store = memory_store();
    let manual = store.add("原创人工保护对象", "人工标记样例", None).unwrap();
    let matched = store
        .add("原创缓存规则触发词", "规则命中样例", None)
        .unwrap();
    let safe = store.add("原创安全可读记录", "普通样例", None).unwrap();
    store
        .capture_files(
            &["/original-cache/notes.txt".into()],
            &CaptureContext::default(),
        )
        .unwrap();
    let image = images::prepare_rgba(1, 1, &[12, 34, 56, 255]).unwrap();
    store
        .add_image(&image, "原创缓存像素", None, false)
        .unwrap();
    let initial = rebuilt_page(&store);
    assert_eq!(initial.history.counts["all"], 5);
    let eligible = || {
        store
            .history_page(&HistoryQuery {
                ai_eligible: true,
                ..HistoryQuery::default()
            })
            .unwrap()
    };
    assert_eq!(eligible().history.total, 3);
    store.set_clip_privacy(manual.id, true, false).unwrap();
    assert_eq!(
        eligible().history.total,
        2,
        "manual protection affects the next read immediately"
    );
    assert!(store
        .history_page(&HistoryQuery {
            privacy: "private".into(),
            ..HistoryQuery::default()
        })
        .unwrap()
        .clips
        .iter()
        .any(|clip| clip.id == manual.id));

    store
        .save_privacy_rules(&PrivacyRules {
            revision: store.privacy_rules().unwrap().revision,
            rules: vec![privacy::PrivacyRule {
                id: "original-cache-rule".into(),
                name: "原创缓存规则".into(),
                enabled: true,
                condition: "keyword".into(),
                value: "缓存规则触发词".into(),
                tag: Some("规则缓存标签".into()),
                mark_private: true,
            }],
        })
        .unwrap();
    assert_eq!(
        eligible().history.total,
        0,
        "a new rule revision cannot reuse previously eligible rows"
    );
    let plan = store.privacy_rescan_preview().unwrap();
    store.privacy_rescan_apply(&plan.token).unwrap();
    let rescanned = eligible();
    assert_eq!(
        rescanned
            .clips
            .iter()
            .map(|clip| clip.id)
            .collect::<Vec<_>>(),
        [safe.id]
    );
    assert!(rescanned.history.tags.contains(&"规则缓存标签".into()));
    assert!(store.clip(matched.id).unwrap().is_private);
    let global_queries = store.history_global_query_count();
    store
        .history_page(&HistoryQuery {
            privacy: "ordinary".into(),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(store.history_global_query_count(), global_queries);
    store.set_clip_privacy(manual.id, false, true).unwrap();
    assert_eq!(
        eligible().history.total,
        2,
        "reviewed unprotection is also visible immediately"
    );
}

#[test]
fn history_cache_does_not_publish_transactional_or_failed_statement_snapshots() {
    let store = memory_store();
    let first = store.add("原创事务缓存甲", "原始事务样例", None).unwrap();
    store.add("原创事务缓存乙", "第二事务样例", None).unwrap();
    store
        .set_clip_tags(first.id, &["已保存标签".into()])
        .unwrap();
    rebuilt_page(&store);
    {
        let tx = store.conn.unchecked_transaction().unwrap();
        tx.execute("UPDATE clips SET pinned=1 WHERE id=?1", [first.id])
            .unwrap();
        let mut metadata = read_metadata(&tx, first.id).unwrap();
        metadata.manual_tags = vec!["未提交标签".into()];
        put_metadata(&tx, first.id, &metadata).unwrap();
        for _ in 0..2 {
            let temporary = rebuilt_page(&store);
            assert_eq!(temporary.history.counts["pinned"], 1);
            assert!(temporary.history.tags.contains(&"未提交标签".into()));
            assert!(
                store.history_global_cache.borrow().is_none(),
                "uncommitted values never become a reusable cache"
            );
        }
        tx.rollback().unwrap();
    }
    let rolled_back = rebuilt_page(&store);
    assert_eq!(rolled_back.history.counts["pinned"], 0);
    assert_eq!(rolled_back.history.tags, ["已保存标签"]);
    {
        let tx = store.conn.unchecked_transaction().unwrap();
        rebuilt_page(&store);
        assert!(
            store.history_global_cache.borrow().is_none(),
            "even a read-only explicit transaction bypasses the cache"
        );
        tx.rollback().unwrap();
    }
    rebuilt_page(&store);
    {
        let tx = store.conn.unchecked_transaction().unwrap();
        tx.execute("UPDATE clips SET keep_forever=1 WHERE id=?1", [first.id])
            .unwrap();
        assert_eq!(rebuilt_page(&store).history.counts["kept"], 1);
        tx.commit().unwrap();
    }
    assert_eq!(rebuilt_page(&store).history.counts["kept"], 1);

    store.conn.execute_batch("CREATE TRIGGER original_partial_cache_failure AFTER UPDATE OF pinned ON clips BEGIN SELECT RAISE(FAIL,'original partial-update fixture'); END;").unwrap();
    assert_eq!(rebuilt_page(&store).history.counts["pinned"], 0);
    let before = store.conn.total_changes();
    assert!(store.conn.execute("UPDATE clips SET pinned=1", []).is_err());
    assert!(store.conn.is_autocommit());
    assert!(
        store.conn.total_changes() > before,
        "RAISE(FAIL) keeps the first changed row despite the error"
    );
    assert_eq!(
        rebuilt_page(&store).history.counts["pinned"],
        1,
        "an error result is not evidence that the database stayed unchanged"
    );
}

struct OriginalDatabase(std::path::PathBuf);
impl OriginalDatabase {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "starclip-history-cache-original-{}-{}",
            std::process::id(),
            CLEANUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for OriginalDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn history_cache_handles_external_commits_active_readers_ddl_and_reopened_stores() {
    let scratch = OriginalDatabase::new();
    let path = scratch.0.join("original.sqlite3");
    let store = Store::open(&path).unwrap();
    store
        .conn
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    let first = store.add("原创外部缓存甲", "外部连接样例", None).unwrap();
    store.add("原创外部缓存乙", "活动游标样例", None).unwrap();
    store
        .set_clip_tags(first.id, &["原始缓存标签".into()])
        .unwrap();
    assert_eq!(rebuilt_page(&store).history.counts["all"], 2);
    let writer = Store::open(&path).unwrap();
    let own_changes = store.conn.total_changes();
    writer
        .set_clip_tags(first.id, &["外部已提交标签".into()])
        .unwrap();
    assert_eq!(store.conn.total_changes(), own_changes);
    assert_eq!(rebuilt_page(&store).history.tags, ["外部已提交标签"]);
    let warm_queries = store.history_global_query_count();
    {
        let tx = writer.conn.unchecked_transaction().unwrap();
        tx.execute("UPDATE clips SET pinned=1", []).unwrap();
        tx.rollback().unwrap();
    }
    assert_eq!(page(&store).history.counts["pinned"], 0);
    assert_eq!(
        store.history_global_query_count(),
        warm_queries,
        "another connection's rollback does not alter the committed snapshot"
    );
    {
        let mut statement = store
            .conn
            .prepare("SELECT id FROM clips ORDER BY id")
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        assert!(rows.next().unwrap().is_some());
        assert!(store.conn.is_autocommit() && store.conn.is_busy());
        writer
            .add("原创外部缓存丙", "WAL期间提交样例", None)
            .unwrap();
        assert_eq!(
            rebuilt_page(&store).history.counts["all"],
            2,
            "an active reader still owns the old WAL snapshot"
        );
        assert!(
            store.history_global_cache.borrow().is_none(),
            "the old reader snapshot must not be published"
        );
    }
    assert_eq!(rebuilt_page(&store).history.counts["all"], 3);
    drop(writer);
    let snapshot_path = scratch.0.join("original-restored-snapshot.sqlite3");
    store.snapshot(&snapshot_path).unwrap();
    let before_ddl = store.conn.total_changes();
    store.conn.execute_batch("DROP TABLE clip_privacy; CREATE TABLE clip_privacy(clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,metadata TEXT NOT NULL);").unwrap();
    assert_eq!(
        store.conn.total_changes(),
        before_ddl,
        "DROP TABLE is not a row-change counter increment"
    );
    let rebuilt = rebuilt_page(&store);
    assert_eq!(rebuilt.history.counts["all"], 3);
    assert!(rebuilt.history.tags.is_empty());
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.history_global_query_count(), 0);
    assert!(reopened.history_global_cache.borrow().is_none());
    assert_eq!(rebuilt_page(&reopened).history.counts["all"], 3);
    assert!(page(&reopened).history.tags.is_empty());
    let restored = Store::open(&snapshot_path).unwrap();
    assert_eq!(restored.history_global_query_count(), 0);
    assert!(restored.history_global_cache.borrow().is_none());
    assert_eq!(rebuilt_page(&restored).history.tags, ["外部已提交标签"]);
    let other = memory_store();
    assert_eq!(other.history_global_query_count(), 0);
    assert_eq!(rebuilt_page(&other).history.counts["all"], 0);
}

#[test]
fn history_cache_returns_complete_large_facets_without_retaining_them() {
    let store = memory_store();
    let tx = store.conn.unchecked_transaction().unwrap();
    for number in 0..1280 {
        let tags: Vec<_> = (0..32)
            .map(|tag| format!("缓存{number:05}{tag:02}{}", "星".repeat(31)))
            .collect();
        privacy::validate_tags(&tags).unwrap();
        tx.execute("INSERT INTO clips(content,title,kind,created_at,updated_at) VALUES(?1,'原创预算样例','text',1,1)", [format!("原创缓存预算内容 {number}")]).unwrap();
        let id = tx.last_insert_rowid();
        let metadata = Metadata {
            record_identity: format!("{id:064x}"),
            scan_version: 1,
            manual_tags: tags,
            ..Metadata::default()
        };
        put_metadata(&tx, id, &metadata).unwrap();
    }
    tx.commit().unwrap();
    let query = HistoryQuery {
        query: "never-present-original-budget-query".into(),
        ..HistoryQuery::default()
    };
    for expected_queries in 1..=2 {
        let result = store.history_page(&query).unwrap();
        assert_eq!(result.history.total, 0);
        assert_eq!(result.history.counts["all"], 1280);
        assert_eq!(
            result.history.tags.len(),
            1280 * 32,
            "the response is complete rather than truncated to fit the cache"
        );
        assert!(result.history.tags.iter().map(String::len).sum::<usize>() > 4 * 1024 * 1024);
        assert!(store.history_global_cache.borrow().is_none());
        assert_eq!(
            store.history_global_query_count(),
            expected_queries,
            "oversized facets are recomputed instead of retained"
        );
    }
}
