//! Frozen old layouts, built forward from schema 0 rather than by relabeling schema 5.
//! These are storage fixtures, not executions of historical application binaries.
use super::*;
use crate::storage::Storage;

const BODY: &str = "fn original_migration_fixture() {}";
const FILES: &str = "/original-migration/one.txt\n/original-migration/folder";

fn legacy_zero(conn: &Connection) {
    conn.execute_batch(r#"PRAGMA foreign_keys=ON;
        CREATE TABLE groups(id INTEGER PRIMARY KEY,name TEXT NOT NULL UNIQUE,color TEXT NOT NULL);
        CREATE TABLE clips(id INTEGER PRIMARY KEY,content TEXT NOT NULL UNIQUE,title TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,copy_count INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE preferences(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        INSERT INTO groups VALUES(7,'Original group','#aabbcc');
        INSERT INTO preferences VALUES('original_opaque','Original opaque preference');
        INSERT INTO preferences VALUES('settings','{"paused":true,"theme":"dark","layout":"grid","maxHistory":0,"retentionDays":0,"shortcut":"CommandOrControl+Shift+V"}');"#).unwrap();
    conn.execute(
        "INSERT INTO clips VALUES(11,?1,'Original title','text',1,7,12,34,9)",
        [BODY],
    )
    .unwrap();
}

// The original image upgrade replaced UNIQUE(content), then privacy v2 added metadata.
// DDL corresponds to the historical layouts in store.rs and the old migration fixtures.
fn advance_to_one(conn: &Connection) {
    conn.execute_batch("ALTER TABLE clips ADD COLUMN language TEXT;
        ALTER TABLE clips ADD COLUMN keep_forever INTEGER NOT NULL DEFAULT 0;
        CREATE TABLE original_upgrade(id INTEGER PRIMARY KEY,content TEXT NOT NULL,title TEXT NOT NULL DEFAULT '',kind TEXT NOT NULL,pinned INTEGER NOT NULL DEFAULT 0,keep_forever INTEGER NOT NULL DEFAULT 0,group_id INTEGER REFERENCES groups(id) ON DELETE SET NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,copy_count INTEGER NOT NULL DEFAULT 0,language TEXT);
        INSERT INTO original_upgrade(id,content,title,kind,pinned,keep_forever,group_id,created_at,updated_at,copy_count,language) SELECT id,content,title,kind,pinned,keep_forever,group_id,created_at,updated_at,copy_count,language FROM clips;
        DROP TABLE clips; ALTER TABLE original_upgrade RENAME TO clips;
        CREATE UNIQUE INDEX clips_text_content ON clips(content) WHERE kind<>'image';
        CREATE UNIQUE INDEX clips_image_content ON clips(content) WHERE kind='image';
        CREATE TABLE clip_images(clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,width INTEGER NOT NULL CHECK(width>0),height INTEGER NOT NULL CHECK(height>0),bytes INTEGER NOT NULL CHECK(bytes>0),png BLOB NOT NULL,thumbnail BLOB NOT NULL);
        UPDATE clips SET kind='code',language='rust',keep_forever=1 WHERE id=11;
        PRAGMA user_version=1;").unwrap();
    let image = picture();
    let m = image.metadata();
    conn.execute(
        "INSERT INTO clips VALUES(12,?1,'Original image','image',1,1,7,14,35,2,NULL)",
        [image.digest()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO clip_images VALUES(12,?1,?2,?3,?4,?5)",
        params![m.width, m.height, m.bytes, image.png, image.thumbnail],
    )
    .unwrap();
}
fn advance_to_two(conn: &Connection) {
    conn.execute_batch("CREATE TABLE clip_privacy(clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,metadata TEXT NOT NULL); PRAGMA user_version=2;").unwrap();
    let metadata = serde_json::json!({"recordIdentity":"11".repeat(32),"privacyManual":true,"manualTags":["Original tag"]});
    conn.execute(
        "INSERT INTO clip_privacy VALUES(11,?1)",
        [metadata.to_string()],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO clips VALUES(13,?1,'Original files','file',1,1,7,16,36,3,NULL)",
        [FILES],
    )
    .unwrap();
}
fn separate_files(conn: &Connection, known: bool) {
    conn.execute_batch("DROP INDEX clips_text_content;
        CREATE UNIQUE INDEX clips_text_content ON clips(content) WHERE kind NOT IN ('image','file');
        CREATE UNIQUE INDEX clips_file_content ON clips(content) WHERE kind='file';
        CREATE TABLE clip_files(clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,position INTEGER NOT NULL CHECK(position>=0),path TEXT NOT NULL,entry_kind TEXT NOT NULL CHECK(entry_kind IN ('file','directory','unknown')),PRIMARY KEY(clip_id,position));
        CREATE UNIQUE INDEX clip_files_path ON clip_files(clip_id,path);").unwrap();
    for (position, path) in FILES.lines().enumerate() {
        let kind = if !known {
            "unknown"
        } else if position == 0 {
            "file"
        } else {
            "directory"
        };
        conn.execute(
            "INSERT INTO clip_files VALUES(13,?1,?2,?3)",
            params![position, path, kind],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO clips VALUES(14,?1,'Original path text','text',1,1,7,18,38,4,NULL)",
        [FILES],
    )
    .unwrap();
}
fn advance_to_four(conn: &Connection, from: u32) {
    if from == 2 {
        separate_files(conn, false);
    }
    conn.execute_batch("CREATE TABLE clip_rich(clip_id INTEGER PRIMARY KEY REFERENCES clips(id) ON DELETE CASCADE,html TEXT NOT NULL);
        INSERT INTO clip_rich VALUES(11,'<p>Original formatted fixture</p>');PRAGMA user_version=4;").unwrap();
    clip_stats::ensure_table(conn).unwrap();
    conn.execute("INSERT INTO clip_activity VALUES(1,'text','new',7,99)", [])
        .unwrap();
    crate::search_index::ensure(conn).unwrap();
}
fn picture() -> PreparedImage {
    images::prepare_rgba(2, 1, &[12, 34, 56, 255, 90, 80, 70, 123]).unwrap()
}
fn tables(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn migration_matrix_covers_each_old_layout_direct_upgrade_and_reopen() {
    for version in 0..=4 {
        let root = super::tests::Scratch::new();
        let mut storage = Storage::open(&root.0, &root.0).unwrap();
        let path = root.0.join("starclip.sqlite3");
        let conn = Connection::open(&path).unwrap();
        legacy_zero(&conn);
        if version >= 1 {
            advance_to_one(&conn);
        }
        if version >= 2 {
            advance_to_two(&conn);
        }
        if version == 3 {
            separate_files(&conn, true);
            conn.pragma_update(None, "user_version", 3).unwrap();
        }
        if version == 4 {
            advance_to_four(&conn, 2);
        }
        let original_tables = tables(&conn);
        let expected_count = if version == 0 {
            1
        } else if version == 1 {
            2
        } else if version == 2 {
            3
        } else {
            4
        };
        assert_eq!(Store::check_schema_version(&conn).unwrap(), version);
        drop(conn);
        fs::create_dir(root.0.join("fonts")).unwrap();
        fs::write(
            root.0.join("fonts/original.fixture"),
            b"Original resource, never executed",
        )
        .unwrap();
        let db = storage.open_store().unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(db.clips().unwrap().len(), expected_count);
        let clip = db.clip(11).unwrap();
        assert_eq!(
            (
                &clip.content,
                &clip.title,
                clip.group_id,
                clip.created_at,
                clip.updated_at,
                clip.copy_count
            ),
            (
                &BODY.to_owned(),
                &"Original title".to_owned(),
                Some(7),
                12,
                34,
                9
            )
        );
        assert!(clip.pinned);
        assert_eq!(clip.keep_forever, version >= 1);
        if version >= 1 {
            assert_eq!(clip.language.as_deref(), Some("rust"));
            assert_eq!(db.image_png(12, false).unwrap(), picture().png);
        }
        if version >= 2 {
            assert!(clip.privacy_manual);
            assert_eq!(clip.record_identity, "11".repeat(32));
            assert!(clip.manual_tags.contains(&"Original tag".into()));
            assert_eq!(db.clip(13).unwrap().entries.unwrap().len(), 2);
        }
        if version >= 3 {
            assert_ne!(db.clip(13).unwrap().id, db.clip(14).unwrap().id);
        }
        if version == 3 {
            assert_eq!(
                db.clip(13).unwrap().entries.unwrap()[1].entry_kind,
                FileEntryKind::Directory
            );
        }
        if version == 4 {
            assert_eq!(
                db.clip_html(11).unwrap().as_deref(),
                Some("<p>Original formatted fixture</p>")
            );
        }
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT value FROM preferences WHERE key='original_opaque'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "Original opaque preference"
        );
        assert!(db.settings().unwrap().paused);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COALESCE(SUM(count),0) FROM clip_activity",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            if version == 4 { 7 } else { 0 },
            "migration preserves activity without inventing events"
        );

        let points = storage.recovery_points().unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].schema_version, version);
        let snapshot = Path::new(&points[0].path);
        let before = Connection::open_with_flags(
            snapshot.join("starclip.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            tables(&before),
            original_tables,
            "the snapshot includes all old tables"
        );
        assert_eq!(Store::check_schema_version(&before).unwrap(), version);
        assert_eq!(
            before
                .query_row("SELECT count(*) FROM clips", [], |r| r.get::<_, usize>(0))
                .unwrap(),
            expected_count
        );
        if version >= 1 {
            assert_eq!(
                before
                    .query_row("SELECT png FROM clip_images WHERE clip_id=12", [], |r| r
                        .get::<_, Vec<u8>>(0))
                    .unwrap(),
                picture().png
            );
        }
        assert_eq!(
            fs::read(snapshot.join("fonts/original.fixture")).unwrap(),
            b"Original resource, never executed"
        );
        let identity = clip.record_identity;
        drop(before);
        assert_eq!(
            storage.cleanup_automatic_recovery_points(u64::MAX).unwrap(),
            1
        );
        assert!(
            !snapshot.exists(),
            "the old snapshot is removed after successful validation"
        );
        assert_eq!(db.clip(11).unwrap().record_identity, identity);
        if version >= 1 {
            assert_eq!(db.image_png(12, false).unwrap(), picture().png);
        }
        drop(db);
        let reopened = storage.open_store().unwrap();
        assert_eq!(reopened.clip(11).unwrap().record_identity, identity);
        assert_eq!(
            storage.recovery_points().unwrap().len(),
            0,
            "no extra snapshot after a completed upgrade"
        );
    }
}

#[test]
fn migration_incremental_known_three_to_four_fixture_then_five_preserves_entries() {
    let root = super::tests::Scratch::new();
    let path = root.0.join("starclip.sqlite3");
    let conn = Connection::open(&path).unwrap();
    legacy_zero(&conn);
    advance_to_one(&conn);
    advance_to_two(&conn);
    separate_files(&conn, true);
    conn.pragma_update(None, "user_version", 3).unwrap();
    assert_eq!(Store::check_schema_version(&conn).unwrap(), 3);
    advance_to_four(&conn, 3);
    assert_eq!(Store::check_schema_version(&conn).unwrap(), 4);
    drop(conn);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
    let file = db.clip(13).unwrap();
    assert_eq!(
        file.entries.unwrap()[1].entry_kind,
        FileEntryKind::Directory
    );
    assert_eq!(db.image_png(12, false).unwrap(), picture().png);
    assert_eq!(db.clip(11).unwrap().record_identity, "11".repeat(32));
    assert_eq!(db.clips().unwrap().len(), 4);
    let mut storage = Storage::open(&root.0, &root.0).unwrap();
    assert_eq!(
        storage.cleanup_automatic_recovery_points(u64::MAX).unwrap(),
        1
    );
    assert!(storage.recovery_points().unwrap().is_empty());
    assert_eq!(db.image_png(12, false).unwrap(), picture().png);
}
