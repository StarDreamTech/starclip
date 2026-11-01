pub mod ai_access;
pub mod ai_transport;
pub mod appearance;
pub mod capture;
pub mod clip_stats;
#[cfg(feature = "desktop")]
mod clipboard_source;
#[cfg(feature = "desktop")]
mod desktop;
pub mod handwriting;
pub mod images;
pub mod privacy;
pub mod search_index;
pub mod share;
pub mod starpet_growth;
#[cfg(feature = "desktop")]
mod startup;
pub mod storage;
pub mod store;
pub mod telemetry;
mod telemetry_device;
#[cfg(feature = "desktop")]
mod updater;
#[cfg(feature = "desktop")]
pub use desktop::run;

#[cfg(test)]
mod capability_tests {
    #[cfg(feature = "desktop")]
    #[test]
    fn generated_runtime_acl_enforces_window_boundaries() {
        // Use the build script's actual manifests and generate_context!'s resolver,
        // without creating an application/window or touching its store/clipboard.
        let manifests = serde_json::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/acl-manifests.json"
        )))
        .unwrap();
        let capabilities =
            serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/capabilities.json")))
                .unwrap();
        let config = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let capabilities =
            tauri::utils::acl::get_capabilities(&config, capabilities, None).unwrap();
        let resolved = tauri::utils::acl::resolved::Resolved::resolve(
            &manifests,
            capabilities,
            tauri::utils::platform::Target::current(),
        )
        .unwrap();
        assert!(
            resolved.has_app_acl,
            "custom commands must go through the runtime ACL gate"
        );
        let authority = tauri::runtime_authority!(manifests, resolved);
        for command in [
            "plugin:autostart|enable",
            "plugin:autostart|disable",
            "plugin:autostart|is_enabled",
            "plugin:updater|check",
            "plugin:updater|download",
            "plugin:updater|install",
            "plugin:updater|download_and_install",
        ] {
            assert!(
                authority
                    .resolve_access(command, "main", "main", &tauri::ipc::Origin::Local)
                    .is_none(),
                "plugin IPC must not bypass application guards"
            );
        }
        let permissions: serde_json::Value =
            serde_json::from_str(include_str!("../permissions/windows.json")).unwrap();
        let main_commands = permissions["permission"]
            .as_array()
            .unwrap()
            .iter()
            .find(|permission| permission["identifier"] == "main-commands")
            .unwrap();
        assert!(main_commands["commands"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .any(|command| command == "open_project_link"));
        for command in [
            "get_startup_preferences",
            "set_startup_preferences",
            "quit_starclip",
            "get_privacy_rules",
            "save_privacy_rules",
            "set_clip_privacy",
            "set_clip_tags",
            "preview_privacy_rescan",
            "apply_privacy_rescan",
            "get_ai_state",
            "set_ai_enabled",
            "create_ai_connection",
            "revoke_ai_connection",
            "get_ai_connection_config",
            "clear_ai_audit",
            "get_appearance",
            "save_appearance",
            "import_appearance_sticker",
            "remove_appearance_sticker",
            "get_telemetry_state",
            "set_telemetry_enabled",
            "record_usage_event",
            "flush_telemetry",
            "clear_telemetry_queue",
            "reset_telemetry_install",
            "get_clip_stats",
            "clear_clip_stats",
        ] {
            assert!(main_commands["commands"]["allow"]
                .as_array()
                .unwrap()
                .iter()
                .any(|allowed| allowed == command));
        }
        for permission in permissions["permission"].as_array().unwrap() {
            let pet_allowed = permission["identifier"] == "starpet-commands";
            for command in permission["commands"]["allow"].as_array().unwrap() {
                let command = command.as_str().unwrap();
                assert!(
                    authority
                        .resolve_access(command, "main", "main", &tauri::ipc::Origin::Local)
                        .is_some(),
                    "main must retain {command}"
                );
                assert_eq!(
                    authority
                        .resolve_access(command, "starpet", "starpet", &tauri::ipc::Origin::Local)
                        .is_some(),
                    pet_allowed,
                    "incorrect starpet access to {command}"
                );
            }
        }
    }

    #[test]
    fn starpet_is_not_granted_history_or_general_window_commands() {
        let pet: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/starpet.json")).unwrap();
        assert_eq!(pet["windows"], serde_json::json!(["starpet"]));
        assert_eq!(
            pet["permissions"],
            serde_json::json!([
                "core:event:allow-listen",
                "core:event:allow-unlisten",
                "starpet-commands"
            ])
        );
        let main: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(main["windows"], serde_json::json!(["main"]));
        let permissions: serde_json::Value =
            serde_json::from_str(include_str!("../permissions/windows.json")).unwrap();
        let pet = permissions["permission"]
            .as_array()
            .unwrap()
            .iter()
            .find(|permission| permission["identifier"] == "starpet-commands")
            .unwrap();
        assert_eq!(
            pet["commands"]["allow"],
            serde_json::json!([
                "get_starpet_preferences",
                "get_starpet_signal",
                "set_starpet_preferences",
                "show_main_window",
                "starpet_start_dragging",
                "starpet_dock",
                "get_starpet_growth",
                "set_starpet_growth_enabled",
                "interact_starpet",
                "equip_starpet_item"
            ])
        );
        assert!(
            permissions.get("default").is_none(),
            "no globally granted custom commands"
        );
    }
}
