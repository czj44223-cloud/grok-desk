mod acp;
mod agent;
mod auth;
mod catalog;
mod cli;
mod plugins;

use tauri::Manager;

#[tauri::command]
fn create_worktree(cwd: String, name: String) -> Result<String, String> {
    catalog::create_worktree(std::path::Path::new(cwd.trim()), &name)
}

#[tauri::command]
fn remove_worktree(cwd: String, id: String, path: String, kind: String) -> Result<(), String> {
    catalog::remove_worktree(std::path::Path::new(cwd.trim()), &id, &path, &kind)
}

#[tauri::command]
fn check_cli() -> cli::CliReport {
    cli::check_cli()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(agent::Agent::new(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_cli,
            agent::desk_info,
            agent::agent_send,
            agent::agent_cancel,
            agent::agent_reply_permission,
            agent::agent_set_project,
            agent::agent_new_session,
            agent::agent_load_session,
            catalog::project_catalog,
            catalog::set_session_archived,
            catalog::session_outputs,
            catalog::preview_file,
            catalog::open_url,
            catalog::open_path,
            agent::delete_session,
            create_worktree,
            remove_worktree,
            auth::auth_status,
            auth::auth_login,
            auth::auth_logout,
            plugins::plugin_catalog,
            plugins::set_plugin_enabled
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                agent::shutdown_from(app);
            }
        });
}
