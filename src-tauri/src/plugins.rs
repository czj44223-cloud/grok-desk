use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::Manager;

const COMPUTER_USE: &str = "computer-use";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRow {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub kind: String,
    pub toggle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginCatalog {
    pub plugins: Vec<PluginRow>,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ListedPlugin {
    name: String,
    description: String,
    enabled: bool,
    kind: String,
}

pub fn plugin_name_ok(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 80
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'))
}

pub fn agent_launch_args(plugin_dirs: &[PathBuf]) -> Vec<String> {
    let mut args = vec!["agent".to_string(), "--no-leader".to_string()];
    for dir in plugin_dirs {
        let text = dir.display().to_string();
        if text.is_empty() {
            continue;
        }
        args.push("--plugin-dir".to_string());
        args.push(text);
    }
    args.push("stdio".to_string());
    args
}

pub fn desktop_plugin_names(text: &str) -> Vec<String> {
    let Ok(names) = serde_json::from_str::<Vec<String>>(text) else {
        return Vec::new();
    };
    names.into_iter().filter(|name| name == COMPUTER_USE).collect()
}

pub fn saved_plugin_dirs(config_dir: &Path) -> Vec<PathBuf> {
    let names = fs::read_to_string(config_dir.join("desktop-plugins.json")).unwrap_or_default();
    if !desktop_plugin_names(&names).iter().any(|name| name == COMPUTER_USE) {
        return Vec::new();
    }
    match prepare_computer_use(config_dir) {
        Ok(dir) => vec![dir],
        Err(_) => Vec::new(),
    }
}

pub fn computer_use_source() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/computer-use"));
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|path| path.to_path_buf());
        for _ in 0..6 {
            let Some(current) = dir else {
                break;
            };
            candidates.push(current.join("plugins").join(COMPUTER_USE));
            dir = current.parent().map(|path| path.to_path_buf());
        }
    }
    for candidate in candidates {
        if candidate.join("plugin.json").is_file() && candidate.join("server.mjs").is_file() {
            return Ok(candidate);
        }
    }
    Err("找不到电脑操作插件的文件。".into())
}

pub fn prepare_computer_use(config_dir: &Path) -> Result<PathBuf, String> {
    let source = computer_use_source()?;
    let dest = config_dir.join("plugins").join(COMPUTER_USE);
    let skill_dir = dest.join("skills").join(COMPUTER_USE);
    fs::create_dir_all(&skill_dir).map_err(|_| "写不了电脑操作插件。".to_string())?;
    for file in ["plugin.json", "server.mjs", "actions.ps1", "computer-use.cmd"] {
        copy_file(&source.join(file), &dest.join(file))?;
    }
    copy_file(
        &source.join("skills").join(COMPUTER_USE).join("SKILL.md"),
        &skill_dir.join("SKILL.md"),
    )?;
    let cmd = dest.join("computer-use.cmd");
    let mcp = mcp_manifest(&cmd, &dest.join("server.mjs"));
    let text = serde_json::to_string_pretty(&mcp).map_err(|_| "写不了电脑操作插件。".to_string())?;
    fs::write(dest.join(".mcp.json"), text).map_err(|_| "写不了电脑操作插件。".to_string())?;
    Ok(dest)
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    if !from.is_file() {
        return Err("电脑操作插件不完整。".into());
    }
    fs::copy(from, to).map(|_| ()).map_err(|_| "写不了电脑操作插件。".to_string())
}

fn mcp_manifest(cmd: &Path, server: &Path) -> Value {
    #[cfg(windows)]
    {
        let _ = server;
        json!({
            "mcpServers": {
                "computer-use": {
                    "command": "cmd.exe",
                    "args": ["/d", "/c", cmd.display().to_string()]
                }
            }
        })
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
        json!({
            "mcpServers": {
                "computer-use": {
                    "command": "node",
                    "args": [server.display().to_string()]
                }
            }
        })
    }
}

fn parse_plugin_list(text: &str) -> Vec<ListedPlugin> {
    let Ok(value) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let items = if let Some(list) = value.as_array() {
        list
    } else {
        let Some(object) = value.as_object() else {
            return Vec::new();
        };
        let Some(list) = ["plugins", "installed", "items", "data"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_array))
        else {
            return Vec::new();
        };
        list
    };
    items.iter().filter_map(listed_plugin).collect()
}

fn listed_plugin(value: &Value) -> Option<ListedPlugin> {
    let name = ["name", "id", "plugin"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() || name == COMPUTER_USE {
        return None;
    }
    let description = value
        .get("description")
        .or_else(|| value.get("desc"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let description: String = description.chars().filter(|ch| !ch.is_control()).take(120).collect();
    let enabled = value
        .get("enabled")
        .and_then(Value::as_bool)
        .or_else(|| {
            value
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status.eq_ignore_ascii_case("enabled"))
        })
        .unwrap_or(false);
    let scope = value.get("scope").and_then(Value::as_str).unwrap_or("");
    Some(ListedPlugin {
        name,
        description,
        enabled,
        kind: kind_label(scope),
    })
}

fn kind_label(scope: &str) -> String {
    match scope.trim() {
        "" | "user" | "cli" => "已安装".to_string(),
        "project" => "项目".to_string(),
        other
            if other.chars().count() <= 20
                && other
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || ch == ' ' || ch == '-' || ch == '_') =>
        {
            other.to_string()
        }
        _ => "已安装".to_string(),
    }
}

fn compose_catalog(computer_enabled: bool, listed: Vec<ListedPlugin>, list_note: &str) -> PluginCatalog {
    let mut plugins = vec![PluginRow {
        name: COMPUTER_USE.to_string(),
        description: "截屏、点击和打字。每次操作仍会先问你。也可以在输入框用 /computer-use。".to_string(),
        enabled: computer_enabled,
        kind: "本窗口".to_string(),
        toggle: true,
    }];
    for item in listed {
        if item.name == COMPUTER_USE || plugins.iter().any(|row| row.name == item.name) {
            continue;
        }
        plugins.push(PluginRow {
            name: item.name.clone(),
            description: item.description,
            enabled: item.enabled,
            kind: item.kind,
            toggle: plugin_name_ok(&item.name),
        });
    }
    let mut note = "电脑操作只在这个窗口里打开，不会写进所有命令行会话。其他插件用本机的 grok plugin 开关。".to_string();
    if !list_note.is_empty() {
        note.push_str(list_note);
    }
    PluginCatalog { plugins, note }
}

fn write_computer_use_flag(path: &Path, enabled: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "写不了插件开关。".to_string())?;
    }
    let names = if enabled { vec![COMPUTER_USE.to_string()] } else { Vec::new() };
    let text = serde_json::to_string(&names).map_err(|_| "写不了插件开关。".to_string())?;
    fs::write(path, text).map_err(|_| "写不了插件开关。".to_string())
}

fn change_message(enabled: bool, reloaded: bool) -> String {
    let state = if enabled { "已打开" } else { "已关闭" };
    if reloaded {
        format!("{state}。这个会话已重新载入。")
    } else {
        format!("{state}。下次开始对话时生效。")
    }
}

#[tauri::command]
pub fn plugin_catalog(app: tauri::AppHandle) -> Result<PluginCatalog, String> {
    let config = app.path().app_config_dir().map_err(|_| "读不了插件开关。".to_string())?;
    let saved = fs::read_to_string(config.join("desktop-plugins.json")).unwrap_or_default();
    let computer_enabled = desktop_plugin_names(&saved).iter().any(|name| name == COMPUTER_USE);
    let (listed, list_note) = match crate::cli::run_grok(&["plugin", "list", "--json"], Duration::from_secs(20)) {
        Ok(captured) if captured.code == 0 => (parse_plugin_list(&captured.stdout), ""),
        Ok(_) => (Vec::new(), " 已安装插件这次没有列出来。"),
        Err(_) => (Vec::new(), " 已安装插件这次没有列出来。"),
    };
    Ok(compose_catalog(computer_enabled, listed, list_note))
}

#[tauri::command]
pub fn set_plugin_enabled(
    app: tauri::AppHandle,
    agent: tauri::State<'_, crate::agent::Agent>,
    name: String,
    enabled: bool,
) -> Result<String, String> {
    let name = name.trim();
    if !plugin_name_ok(name) {
        return Err("这个插件名字不能从这里开关。".into());
    }
    agent.ensure_idle()?;
    if name == COMPUTER_USE {
        let config = app.path().app_config_dir().map_err(|_| "写不了插件开关。".to_string())?;
        let dirs = if enabled {
            vec![prepare_computer_use(&config)?]
        } else {
            Vec::new()
        };
        write_computer_use_flag(&config.join("desktop-plugins.json"), enabled)?;
        let reloaded = agent.apply_plugin_dirs(dirs)?;
        return Ok(change_message(enabled, reloaded));
    }
    let verb = if enabled { "enable" } else { "disable" };
    let captured = crate::cli::run_grok(&["plugin", verb, name], Duration::from_secs(30))?;
    if captured.code != 0 {
        return Err("插件开关没有完成。".into());
    }
    let dirs = agent.plugin_dirs()?;
    let reloaded = agent.apply_plugin_dirs(dirs)?;
    Ok(change_message(enabled, reloaded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_put_the_plugin_dir_before_stdio_and_never_auto_approve() {
        let args = agent_launch_args(&[PathBuf::from(r"D:\apps\grok-desk\plugins\computer-use")]);
        assert_eq!(
            args,
            vec![
                "agent".to_string(),
                "--no-leader".to_string(),
                "--plugin-dir".to_string(),
                r"D:\apps\grok-desk\plugins\computer-use".to_string(),
                "stdio".to_string(),
            ]
        );
        assert!(args.iter().all(|arg| arg != "--always-approve" && !arg.contains("yolo")));
        assert_eq!(
            agent_launch_args(&[]),
            vec!["agent".to_string(), "--no-leader".to_string(), "stdio".to_string()]
        );
    }

    #[test]
    fn plugin_names_reject_paths_and_blank_text() {
        assert!(plugin_name_ok("computer-use"));
        assert!(plugin_name_ok("team.tools:one"));
        assert!(!plugin_name_ok("user/hash/name"));
        assert!(!plugin_name_ok("../plugin"));
        assert!(!plugin_name_ok(""));
        assert!(!plugin_name_ok("has space"));
    }

    #[test]
    fn catalog_keeps_computer_use_first_and_hides_unsafe_toggles() {
        let listed = parse_plugin_list(
            r#"{"plugins":[
                {"name":"computer-use","description":"重复","enabled":true},
                {"id":"user/hash/name","description":"不能开关","enabled":true},
                {"name":"gdrive","description":"网盘","enabled":false,"scope":"user"}
            ]}"#,
        );
        assert_eq!(listed.len(), 2);
        let catalog = compose_catalog(false, listed, "");
        assert_eq!(catalog.plugins[0].name, "computer-use");
        assert!(!catalog.plugins[0].enabled);
        assert_eq!(catalog.plugins[0].kind, "本窗口");
        let blocked = catalog.plugins.iter().find(|row| row.name.contains('/')).unwrap();
        assert!(!blocked.toggle);
        let gdrive = catalog.plugins.iter().find(|row| row.name == "gdrive").unwrap();
        assert!(gdrive.toggle);
        assert!(!gdrive.enabled);
        assert!(catalog.note.contains("这个窗口"));
    }

    #[test]
    fn prepared_copy_points_mcp_at_an_absolute_command() {
        let dir = std::env::temp_dir().join(format!(
            "grok-desk-cu-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let prepared = prepare_computer_use(&dir).unwrap();
        let text = fs::read_to_string(prepared.join(".mcp.json")).unwrap();
        assert!(!text.contains("CLAUDE_PLUGIN_ROOT"));
        assert!(!text.contains("GROK_PLUGIN_ROOT"));
        let value: Value = serde_json::from_str(&text).unwrap();
        let args = value["mcpServers"]["computer-use"]["args"].as_array().unwrap();
        let command = args.last().and_then(Value::as_str).unwrap();
        let command_path = PathBuf::from(command);
        assert!(command_path.is_absolute());
        assert!(command_path.is_file());
        assert!(command_path.ends_with("computer-use.cmd") || command_path.ends_with("server.mjs"));
        let flag = dir.join("desktop-plugins.json");
        write_computer_use_flag(&flag, true).unwrap();
        assert_eq!(desktop_plugin_names(&fs::read_to_string(&flag).unwrap()), vec!["computer-use".to_string()]);
        write_computer_use_flag(&flag, false).unwrap();
        assert!(desktop_plugin_names(&fs::read_to_string(&flag).unwrap()).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
