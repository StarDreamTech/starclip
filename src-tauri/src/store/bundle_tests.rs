use super::*;

fn memory() -> Store {
    Store::from_connection(Connection::open_in_memory().unwrap()).unwrap()
}
fn picture(value: u8) -> PreparedImage {
    images::prepare_rgba(2, 1, &[value, 40, 90, 255, value, 80, 50, 127]).unwrap()
}

#[test]
fn schema_five_migrates_four_with_snapshot_and_preserves_original_identity() {
    let root = std::env::temp_dir().join(format!(
        "starclip-original-bundle-migration-{}-{}",
        std::process::id(),
        now()
    ));
    fs::create_dir(&root).unwrap();
    let path = root.join("starclip.sqlite3");
    let db = Store::open(&path).unwrap();
    let image = picture(19);
    let clip = db.add_image(&image, "original four", None, true).unwrap();
    db.set_clip_privacy(clip.id, true, false).unwrap();
    let backup = db.export().unwrap();
    db.conn.execute_batch("DROP TRIGGER bundle_images_cleanup; DROP TABLE clip_bundle_items; DROP TABLE clip_bundles; DROP TABLE bundle_images;
        DROP INDEX clips_image_content; CREATE UNIQUE INDEX clips_image_content ON clips(content) WHERE kind='image';
        DROP INDEX clips_text_content; CREATE UNIQUE INDEX clips_text_content ON clips(content) WHERE kind NOT IN ('image','file'); PRAGMA user_version=4;").unwrap();
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.schema_version().unwrap(), 5);
    assert_eq!(db.add_image(&image, "", None, false).unwrap().id, clip.id);
    let current = db.clip(clip.id).unwrap();
    assert_eq!(current.record_identity, clip.record_identity);
    assert!(current.keep_forever && current.privacy_manual);
    assert_eq!(db.image_png(clip.id, false).unwrap(), image.png);
    assert!(fs::read_dir(root.join("recovery"))
        .unwrap()
        .next()
        .is_some());
    let mut target = memory();
    let mut old: serde_json::Value = serde_json::from_str(&backup).unwrap();
    old["version"] = 4.into();
    assert_eq!(target.import(&old.to_string()).unwrap(), 1);
    // A future structure is rejected before opening a writable connection.
    db.conn.execute_batch("PRAGMA user_version=6").unwrap();
    drop(db);
    assert!(Store::open(&path).err().unwrap().contains("较新版本"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_edits_preserve_images_protection_and_reject_duplicate_identity() {
    let db = memory();
    let a = picture(10);
    let b = picture(11);
    let first = db
        .add_bundle("original caption", &[a, b], "original", true, true, None)
        .unwrap();
    let group = db.create_group("original group", "#aabbcc").unwrap();
    let png = db.image_member_png(first.id, 1, false).unwrap();
    db.update_bundle(
        first.id,
        "new original caption",
        "edited",
        Some(group.id),
        true,
    )
    .unwrap();
    assert_eq!(db.image_member_png(first.id, 1, false).unwrap(), png);
    assert!(db.clip(first.id).unwrap().privacy_manual);
    let other = db
        .add_bundle(
            "different caption",
            &[picture(10), picture(11)],
            "other",
            false,
            false,
            None,
        )
        .unwrap();
    assert!(db
        .update_bundle(other.id, "new original caption", "", None, false)
        .is_err());
    let combined = db
        .bundle_from_clips(&[first.id], "extra original caption", "", true)
        .unwrap();
    assert_eq!(
        combined.content,
        "new original caption\n\nextra original caption"
    );
    assert!(combined.privacy_manual && combined.pinned);
    assert_eq!(combined.images.len(), 2);
    assert!(db.image_member_png(combined.id, 2, false).is_err());
}

#[test]
fn image_dedup_exact_bytes_pixels_and_collision_buckets_preserve_every_original() {
    let db = memory();
    let a = picture(12);
    let mut b = picture(44);
    b.force_test_digest(a.digest());
    let first = db.add_image(&a, "original A", None, true).unwrap();
    db.set_clip_privacy(first.id, true, false).unwrap();
    let second = db.add_image(&b, "original B", None, false).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(db.add_image(&a, "", None, false).unwrap().id, first.id);
    assert_eq!(db.add_image(&b, "", None, false).unwrap().id, second.id);
    assert_eq!(db.image_png(first.id, false).unwrap(), a.png);
    assert_eq!(db.image_png(second.id, false).unwrap(), b.png);
    assert!(db.clip(first.id).unwrap().privacy_manual && db.clip(first.id).unwrap().keep_forever);
    assert_eq!(db.clips().unwrap().len(), 2);
}

#[test]
fn image_dedup_import_verifies_candidate_and_does_not_overwrite_on_digest_match() {
    let mut destination = memory();
    let original = picture(14);
    let incoming = picture(78);
    let first = destination
        .add_image(&original, "original protected", None, true)
        .unwrap();
    // Simulates a digest collision without attempting to manufacture SHA-256 collisions.
    destination
        .conn
        .execute(
            "UPDATE clips SET content=?2 WHERE id=?1",
            params![first.id, incoming.digest()],
        )
        .unwrap();
    let source = memory();
    source
        .add_image(&incoming, "original import", None, false)
        .unwrap();
    let backup = source.export().unwrap();
    assert_eq!(destination.import(&backup).unwrap(), 1);
    assert_eq!(destination.import(&backup).unwrap(), 0);
    assert_eq!(
        destination.image_png(first.id, false).unwrap(),
        original.png
    );
    assert_eq!(destination.clips().unwrap().len(), 2);
}

#[test]
fn bundle_text_order_repeated_members_dedup_assets_and_backup_roundtrip() {
    let db = memory();
    let pictures = vec![picture(20), picture(40)];
    let first = db
        .add_bundle(
            "Original caption",
            &pictures,
            "Original album",
            true,
            true,
            None,
        )
        .unwrap();
    assert_eq!(first.kind, "bundle");
    assert_eq!(first.images.len(), 2);
    assert!(first.privacy_manual && first.pinned);
    assert_eq!(
        db.add_bundle("Original caption", &pictures, "", false, false, None)
            .unwrap()
            .id,
        first.id
    );
    let reverse = vec![picture(40), picture(20)];
    let other = db
        .add_bundle("Original caption", &reverse, "", false, false, None)
        .unwrap();
    assert_ne!(first.id, other.id);
    let repeated = vec![picture(20), picture(20)];
    let double = db
        .add_bundle("", &repeated, "", false, false, None)
        .unwrap();
    assert_eq!(double.images.len(), 2);
    assert_eq!(
        db.image_member_png(double.id, 0, false).unwrap(),
        db.image_member_png(double.id, 1, false).unwrap()
    );
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM bundle_images", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    let backup = db.export().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&backup).unwrap()["version"],
        5
    );
    let mut restored = memory();
    assert_eq!(restored.import(&backup).unwrap(), 3);
    assert_eq!(restored.import(&backup).unwrap(), 0);
    let clips = restored.clips().unwrap();
    assert_eq!(clips.len(), 3);
    let saved = clips.iter().find(|c| c.title == "Original album").unwrap();
    assert!(saved.pinned && saved.privacy_manual);
    assert_eq!(
        restored.image_member_png(saved.id, 1, false).unwrap(),
        pictures[1].png
    );
    db.delete(other.id).unwrap();
    db.delete(double.id).unwrap();
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM bundle_images", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    db.delete(first.id).unwrap();
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM bundle_images", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn bundle_private_limits_ai_and_image_filter_are_enforced() {
    let db = memory();
    let image = db
        .add_image(&picture(22), "Original private", None, true)
        .unwrap();
    db.set_clip_privacy(image.id, true, false).unwrap();
    assert!(db
        .bundle_from_clips(&[image.id], "caption", "", false)
        .is_err());
    let album = db
        .bundle_from_clips(&[image.id], "caption", "", true)
        .unwrap();
    assert!(album.privacy_manual && album.pinned);
    assert!(db
        .history_page(&HistoryQuery {
            ai_eligible: true,
            ..Default::default()
        })
        .unwrap()
        .clips
        .is_empty());
    let state = db
        .history_page(&HistoryQuery {
            filter: "image".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(state.clips.len(), 2);
    let over = (0..51).map(|i| picture(i)).collect::<Vec<_>>();
    assert!(db.add_bundle("", &over, "", false, false, None).is_err());
    let backup = db.export().unwrap();
    let mut invalid: serde_json::Value = serde_json::from_str(&backup).unwrap();
    invalid["bundles"][0]["images"]
        .as_array_mut()
        .unwrap()
        .clear();
    let mut clean = memory();
    assert!(clean.import(&invalid.to_string()).is_err());
    assert!(clean.clips().unwrap().is_empty());
}
