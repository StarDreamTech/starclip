use super::{project_link, write_and_suppress};
use crate::capture::{CaptureKey, CaptureState};

#[test]
fn startup_native_operations_release_storage_and_rollback_after_update_freeze() {
    use super::*;
    use std::cell::{Cell, RefCell};

    for freeze_during_apply in [false, true] {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "starclip-startup-lock-test-{:x}",
            u128::from_le_bytes(random)
        ));
        fs::create_dir_all(&path).unwrap();
        let storage = Storage::open(&path, &path).unwrap();
        let store = storage.open_store().unwrap();
        let previous = store.window_startup_preferences().unwrap();
        let context = Context {
            telemetry: Mutex::new(Telemetry::new(Ok(Default::default()))),
            storage: Mutex::new(storage),
            store: Mutex::new(store),
            ai: Mutex::new(AiAccess::default()),
            reader: Mutex::new(Reader {
                clipboard: None,
                capture: CaptureState::default(),
            }),
            clipboard_error: Mutex::new(None),
            shortcut_error: Mutex::new(None),
            window_error: Mutex::new(None),
            cleanup: Mutex::new(None),
            starpet_operation: Mutex::new(()),
            startup_operation: Mutex::new(()),
            tray_items: OnceLock::new(),
            native_language: Mutex::new("zh-CN".into()),
        };
        let auto_launch = Cell::new(false);
        let tray_visible = Cell::new(true);
        let operations = RefCell::new(Vec::new());
        let result = apply_startup_preferences(
            &context,
            StartupPreferencePatch {
                auto_launch: Some(true),
                start_hidden: Some(true),
                show_tray_icon: Some(false),
            },
            || {
                assert!(
                    context.storage.try_lock().is_ok(),
                    "native read holds storage"
                );
                Ok(auto_launch.get())
            },
            |enabled| {
                // A synchronous native callback must be able to acquire
                // storage, both during application and during rollback.
                assert!(
                    context.storage.try_lock().is_ok(),
                    "login operation holds storage"
                );
                auto_launch.set(enabled);
                operations.borrow_mut().push(("login", enabled));
                Ok(())
            },
            |visible| {
                let mut storage = context
                    .storage
                    .try_lock()
                    .expect("tray operation holds storage");
                tray_visible.set(visible);
                operations.borrow_mut().push(("tray", visible));
                if freeze_during_apply && !visible {
                    storage.freeze_for_update().unwrap();
                }
                Ok(())
            },
        );
        let saved = context
            .store
            .lock()
            .unwrap()
            .window_startup_preferences()
            .unwrap();
        if freeze_during_apply {
            assert!(result
                .err()
                .expect("freeze must reject persistence")
                .contains("正在安装更新"));
            assert_eq!(
                saved, previous,
                "failed save must retain the original preferences"
            );
            assert!(!auto_launch.get());
            assert!(tray_visible.get());
            assert_eq!(
                *operations.borrow(),
                [
                    ("login", true),
                    ("tray", false),
                    ("login", false),
                    ("tray", true)
                ]
            );
            assert!(context.storage.lock().unwrap().writable().is_err());
        } else {
            let result = result.unwrap();
            assert!(result.auto_launch && result.start_hidden && !result.show_tray_icon);
            assert!(saved.start_hidden && !saved.show_tray_icon);
            assert!(auto_launch.get());
            assert!(!tray_visible.get());
            assert_eq!(*operations.borrow(), [("login", true), ("tray", false)]);
        }
        assert!(context.startup_operation.try_lock().is_ok());
        drop(context);
        fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn settings_interactions_notify_only_after_validation_without_double_rewards_or_enabling_pet() {
    use super::{
        ai_access, apply_starpet_interaction, AiAccess, CreateConnection, GrowthChange, Store,
    };
    use serde_json::json;
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    let path = std::env::temp_dir().join(format!(
        "starclip-action-sync-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&path).unwrap();
    let store = Store::open(&path.join("fixed-fixture.sqlite3")).unwrap();
    let mut preferences = store.starpet_preferences().unwrap();
    preferences.enabled = true;
    preferences.mood = "reading".into();
    store.save_starpet_preferences(&preferences).unwrap();
    store
        .change_starpet_growth(GrowthChange::Enabled(true))
        .unwrap();
    let mut ai = AiAccess::default();
    ai.port = Some(12345);
    ai.enable(true).unwrap();
    let config = ai
        .create(
            &store,
            CreateConnection {
                name: "原创动作同步测试".into(),
                clip_ids: vec![],
                fields: vec![],
                max_results: 5,
                expires_hours: 1,
                allow_pet_control: true,
            },
            Path::new("/fixed-fixture/starclip"),
        )
        .unwrap();
    let token = config["config"]["mcp"]["mcpServers"]["starclip"]["env"]["STARCLIP_AI_TOKEN"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        ai.respond(
            &store,
            ai_access::Request {
                token,
                action: "pet_signal".into(),
                arguments: json!({"status":"working","message":"原创临时状态"})
            }
        )["result"]["accepted"],
        true
    );
    let before = store.starpet_growth().unwrap();
    let signal_revision = ai.pet_snapshot().revision;
    for action in ["twirl", "not-an-action"] {
        assert!(apply_starpet_interaction(&store, &mut ai, action, true).is_err());
        assert_eq!(store.starpet_growth().unwrap(), before);
        assert!(
            ai.pet_snapshot().signal.is_some(),
            "rejected action must not clear the Agent signal"
        );
    }
    let (greeted, event) = apply_starpet_interaction(&store, &mut ai, "greet", true).unwrap();
    let event = event.unwrap();
    assert_eq!(greeted.xp, 10);
    assert_eq!(event.action, "greet");
    assert!(event.animate);
    assert!(event.signal_revision > signal_revision);
    assert!(ai.pet_snapshot().signal.is_none());
    let (repeated, again) = apply_starpet_interaction(&store, &mut ai, "greet", true).unwrap();
    assert_eq!(repeated.xp, greeted.xp);
    assert_eq!(
        again.unwrap().action,
        "greet",
        "identical actions still produce a fresh playback event"
    );
    let (_, touch) = apply_starpet_interaction(&store, &mut ai, "greet", false).unwrap();
    assert!(!touch.unwrap().animate);
    assert_eq!(
        store.starpet_preferences().unwrap(),
        preferences,
        "interactions do not change the saved mood or appearance"
    );
    preferences.enabled = false;
    store.save_starpet_preferences(&preferences).unwrap();
    store
        .change_starpet_growth(GrowthChange::Enabled(false))
        .unwrap();
    let (paused, absent) = apply_starpet_interaction(&store, &mut ai, "highfive", true).unwrap();
    assert!(absent.is_none());
    assert_eq!(paused.xp, greeted.xp);
    assert_eq!(store.starpet_preferences().unwrap(), preferences);
    drop(store);
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn reopening_invalidates_delayed_main_window_release_requests() {
    let session = super::MainWindowSession::default();
    assert_eq!(session.pending_request().unwrap(), None);
    let close = session.next_request(true).unwrap();
    assert!(session.is_current(close).unwrap());
    assert_eq!(session.pending_request().unwrap(), Some(close));

    // A Dock / tray / shortcut reopen invalidates an old frontend reply
    // before it queues a native show, even when the worker is still busy.
    let reopen = session.next_request(false).unwrap();
    assert!(!session.is_current(close).unwrap());
    assert!(!session.is_current(reopen).unwrap());
    assert_eq!(session.pending_request().unwrap(), None);

    let next_close = session.next_request(true).unwrap();
    assert!(!session.is_current(reopen).unwrap());
    assert!(session.is_current(next_close).unwrap());
    // A late completion cannot clear a newer pending close.
    session.finish_request(close).unwrap();
    assert_eq!(session.pending_request().unwrap(), Some(next_close));
    session.finish_request(next_close).unwrap();
    assert_eq!(session.pending_request().unwrap(), None);
}

#[test]
fn ai_broker_checks_privacy_revoke_freeze_and_concurrent_changes() {
    use super::*;
    use serde_json::json;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).unwrap();
    let path = std::env::temp_dir().join(format!(
        "starclip-broker-test-{:x}",
        u128::from_le_bytes(random)
    ));
    fs::create_dir_all(&path).unwrap();
    let storage = Storage::open(&path, &path).unwrap();
    let store = storage.open_store().unwrap();
    let clip = store
        .add("原创星图 https://linux.do/fixed", "原创测试", None)
        .unwrap();
    store.set_clip_tags(clip.id, &["星图".into()]).unwrap();
    let context = Arc::new(Context {
        telemetry: Mutex::new(Telemetry::new(Ok(Default::default()))),
        storage: Mutex::new(storage),
        store: Mutex::new(store),
        ai: Mutex::new(AiAccess::default()),
        reader: Mutex::new(Reader {
            clipboard: None,
            capture: CaptureState::default(),
        }),
        clipboard_error: Mutex::new(None),
        shortcut_error: Mutex::new(None),
        window_error: Mutex::new(None),
        cleanup: Mutex::new(None),
        starpet_operation: Mutex::new(()),
        startup_operation: Mutex::new(()),
        tray_items: OnceLock::new(),
        native_language: Mutex::new("zh-CN".into()),
    });
    let (pet_events, received_pet_events) = std::sync::mpsc::channel();
    start_ai_broker(&context, move |snapshot| {
        pet_events.send(snapshot.clone()).map_err(|e| e.to_string())
    });
    let port = context.ai.lock().unwrap().port.unwrap();
    let store = context.store.lock().unwrap();
    let mut ai = context.ai.lock().unwrap();
    ai.enable(true).unwrap();
    let config = ai
        .create(
            &store,
            CreateConnection {
                name: "原创测试".into(),
                clip_ids: vec![clip.id],
                fields: vec![
                    "title".into(),
                    "content".into(),
                    "tags".into(),
                    "sources".into(),
                ],
                max_results: 5,
                expires_hours: 1,
                allow_pet_control: false,
            },
            Path::new("/fixture/starclip"),
        )
        .unwrap();
    let token = config["config"]["mcp"]["mcpServers"]["starclip"]["env"]["STARCLIP_AI_TOKEN"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = config["connectionId"].as_str().unwrap().to_owned();
    drop(ai);
    drop(store);
    // TCP frames can arrive in separate packets. A partial authorized
    // request must wait for its newline, not be treated as a denial.
    let mut fragmented = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    fragmented.set_nodelay(true).unwrap();
    fragmented
        .set_read_timeout(Some(Duration::from_millis(150)))
        .unwrap();
    let frame = serde_json::to_vec(&json!({
        "token": token,
        "action": "search",
        "arguments": {}
    }))
    .unwrap();
    let split = frame.len() / 2;
    fragmented.write_all(&frame[..split]).unwrap();
    let early = fragmented.peek(&mut [0u8; 1]).unwrap_err();
    assert!(matches!(
        early.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ));
    fragmented
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    fragmented.write_all(&frame[split..]).unwrap();
    fragmented.write_all(b"\n").unwrap();
    let reply = ai_access::read_frame(
        &mut std::io::BufReader::new(fragmented),
        ai_access::MAX_RESPONSE,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&reply).unwrap()["result"]["total"],
        1
    );
    assert_eq!(
        ai_access::client(port, &token, "search", json!({})).unwrap()["total"],
        1
    );
    // The request cannot emit data while a privacy mutation owns the store.
    let store = context.store.lock().unwrap();
    let worker_token = token.clone();
    let worker = thread::spawn(move || {
        ai_access::client(port, &worker_token, "get", json!({"id": clip.id}))
    });
    store.set_clip_privacy(clip.id, true, false).unwrap();
    drop(store);
    assert_eq!(worker.join().unwrap().unwrap_err(), ai_access::DENIED);
    assert_eq!(
        ai_access::client(port, &token, "search", json!({})).unwrap()["total"],
        0
    );
    for action in ["list_sources", "list_tags"] {
        assert_eq!(
            ai_access::client(port, &token, action, json!({})).unwrap()["returned"],
            0
        );
    }
    context.storage.lock().unwrap().freeze_for_update().unwrap();
    assert!(ai_access::client(port, &token, "status", json!({})).is_err());
    context.storage.lock().unwrap().cancel_update_freeze();
    context.ai.lock().unwrap().revoke(&id);
    for action in ai_access::TOOLS {
        let args = if action == "get" {
            json!({"id": clip.id})
        } else {
            json!({})
        };
        assert!(ai_access::client(port, &token, action, args).is_err());
    }
    // Exercise the real loopback-to-pet callback with synthetic data only.
    // This creates no native window and never opens a system clipboard.
    let pet_config = {
        let store = context.store.lock().unwrap();
        let mut preferences = store.starpet_preferences().unwrap();
        preferences.enabled = true;
        store.save_starpet_preferences(&preferences).unwrap();
        context
            .ai
            .lock()
            .unwrap()
            .create(
                &store,
                CreateConnection {
                    name: "原创 Agent 测试".into(),
                    clip_ids: vec![],
                    fields: vec![],
                    max_results: 5,
                    expires_hours: 1,
                    allow_pet_control: true,
                },
                Path::new("/fixture/starclip"),
            )
            .unwrap()
    };
    let pet_token = pet_config["config"]["mcp"]["mcpServers"]["starclip"]["env"]
        ["STARCLIP_AI_TOKEN"]
        .as_str()
        .unwrap();
    assert!(ai_access::client(port, pet_token, "search", json!({})).is_err());
    assert_eq!(
        ai_access::client(
            port,
            pet_token,
            "pet_signal",
            json!({"status":"working","action":"greet","message":"原创测试进度"})
        )
        .unwrap()["accepted"],
        true
    );
    let signal = received_pet_events
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(
        signal.signal.unwrap().message.as_deref(),
        Some("原创测试进度")
    );
    assert_eq!(
        ai_access::client(port, pet_token, "pet_clear", json!({})).unwrap()["cleared"],
        true
    );
    assert!(received_pet_events
        .recv_timeout(Duration::from_secs(1))
        .unwrap()
        .signal
        .is_none());
    context.ai.lock().unwrap().enable(false).unwrap();
    assert!(ai_access::client(port, &token, "status", json!({})).is_err());
    drop(context);
    let _ = fs::remove_dir_all(path);
}

#[test]
fn shared_clipboard_writer_suppresses_only_successful_own_writes() {
    let mut capture = CaptureState::default();
    capture.accept(
        Some(1),
        Some(CaptureKey::Text("fixed previous text".into())),
    );
    let text = CaptureKey::Text("fixed product copy".into());
    write_and_suppress(&mut capture, text.clone(), || Ok(())).unwrap();
    assert!(capture.should_read(Some(2)));
    assert!(!capture.needs_capture(&Some(text.clone())));
    let image = CaptureKey::Image("fixed image digest".into());
    assert!(write_and_suppress(
        &mut capture,
        image.clone(),
        || Err("fixture failure".into())
    )
    .is_err());
    assert!(!capture.needs_capture(&Some(text)));
    assert!(capture.needs_capture(&Some(image.clone())));
    write_and_suppress(&mut capture, image.clone(), || Ok(())).unwrap();
    assert!(!capture.needs_capture(&Some(image)));
    assert!(capture.needs_capture(&Some(CaptureKey::Text("later external text".into()))));
}

#[test]
fn project_links_only_accept_fixed_targets_without_opening_a_browser() {
    assert_eq!(
        project_link("repository").unwrap(),
        "https://github.com/cpython666/starclip"
    );
    assert_eq!(
        project_link("issues").unwrap(),
        "https://github.com/cpython666/starclip/issues/new"
    );
    for target in [
        "",
        "Repository",
        " issues",
        "repository/../../",
        "https://github.com/cpython666/starclip",
        "https://example.com",
        "file:///tmp/test",
        "javascript:alert(1)",
        "issues;open example.com",
    ] {
        assert!(project_link(target).is_err(), "must reject {target:?}");
    }
}
