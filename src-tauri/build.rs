fn main() {
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(&[
            "check_cli",
            "desk_info",
            "agent_send",
            "agent_cancel",
            "agent_reply_permission",
            "agent_set_project",
            "agent_new_session",
            "agent_load_session",
            "project_catalog",
            "set_session_archived",
            "session_outputs",
            "preview_file",
            "open_url",
            "open_path",
            "delete_session",
            "create_worktree",
            "remove_worktree",
            "auth_status",
            "auth_login",
            "auth_logout",
            "plugin_catalog",
            "set_plugin_enabled",
        ]));
    tauri_build::try_build(attributes).expect("failed to run build script");
}
