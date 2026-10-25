use std::fs;
use std::io::{BufRead, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub updated_at: String,
    pub model: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    pub name: String,
    pub description: String,
    pub scope: String,
    pub invocable: bool,
    pub hint: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookRow {
    pub scope: String,
    pub event: String,
    pub matcher: String,
    pub kind: String,
    pub target: String,
    pub runs: bool,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeRow {
    pub id: String,
    pub path: String,
    pub label: String,
    pub branch: String,
    pub status: String,
    pub kind: String,
    pub removable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub kind: String,
    pub isolation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub sessions: Vec<SessionRow>,
    pub skills: Vec<SkillRow>,
    pub hooks: Vec<HookRow>,
    pub trust_note: String,
    pub worktrees: Vec<WorktreeRow>,
    pub worktree_note: String,
    pub subagents: Vec<SubagentRow>,
    pub recent: Vec<String>,
}

pub struct CatalogInput<'a> {
    pub grok_home: &'a Path,
    pub cwd: &'a Path,
    pub session_id: &'a str,
    pub trust_text: &'a str,
    pub worktree_json: Option<&'a str>,
    pub git_porcelain: Option<&'a str>,
}

/// Grok names a session group by percent-encoding the working directory.
/// Characters outside ASCII letters, digits, and `-_.~` become `%HH`.
pub fn encode_cwd(cwd: &str) -> String {
    let mut out = String::new();
    for ch in cwd.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for byte in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

pub fn build_catalog(input: &CatalogInput<'_>) -> Catalog {
    let home = user_home();
    let sessions = list_sessions(input.grok_home, input.cwd);
    let skills = list_skills(input.grok_home, &home, input.cwd);
    let hooks = list_hooks(input.grok_home, &home, input.cwd);
    let has_project = skills.iter().any(|skill| skill.scope != "个人" && skill.scope != "内置")
        || hooks.iter().any(|hook| hook.scope == "项目");
    let worktrees = merge_listed(
        input.cwd,
        input
            .worktree_json
            .map(parse_grok_worktrees)
            .unwrap_or_default(),
        input
            .git_porcelain
            .map(|text| parse_git_worktrees(input.cwd, text))
            .unwrap_or_default(),
    );
    let subagents = if input.session_id.is_empty() {
        Vec::new()
    } else {
        list_subagents(input.grok_home, input.cwd, input.session_id)
    };
    Catalog {
        sessions,
        skills,
        hooks,
        trust_note: trust_note(input.cwd, input.trust_text, has_project),
        worktrees,
        worktree_note: String::new(),
        subagents,
        recent: Vec::new(),
    }
}

pub fn list_sessions(grok_home: &Path, cwd: &Path) -> Vec<SessionRow> {
    let Some(group) = session_group(&grok_home.join("sessions"), cwd) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    let Ok(entries) = fs::read_dir(&group) else {
        return rows;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(row) = read_summary(&path) {
            rows.push(row);
        }
    }
    rows.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    rows.truncate(80);
    rows
}

fn session_group(root: &Path, cwd: &Path) -> Option<PathBuf> {
    let cwd_text = cwd.display().to_string();
    let direct = root.join(encode_cwd(&cwd_text));
    if direct.is_dir() {
        return Some(direct);
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let marker = entry.path().join(".cwd");
        if let Ok(text) = fs::read_to_string(marker) {
            if paths_eq(text.trim(), &cwd_text) {
                return Some(entry.path());
            }
        }
    }
    None
}

fn read_summary(dir: &Path) -> Option<SessionRow> {
    let text = fs::read_to_string(dir.join("summary.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let folder = dir.file_name().and_then(|name| name.to_str()).unwrap_or("");
    let id = text_at(&value, &["info", "id"]);
    let id = if id.is_empty() { folder.to_string() } else { id };
    if id.is_empty() {
        return None;
    }
    let generated = text_at(&value, &["generated_title"]);
    let summary = text_at(&value, &["session_summary"]);
    let last = text_at(&value, &["last_turn_summary"]);
    let title = if !generated.is_empty() {
        generated
    } else if !summary.is_empty() {
        summary.clone()
    } else {
        "未命名会话".to_string()
    };
    let summary = if !summary.is_empty() { summary } else { last };
    Some(SessionRow {
        id,
        title,
        summary,
        updated_at: text_at(&value, &["updated_at"]),
        model: text_at(&value, &["current_model_id"]),
        archived: false,
    })
}

pub fn list_skills(grok_home: &Path, home: &Path, cwd: &Path) -> Vec<SkillRow> {
    let mut found: Vec<(i32, SkillRow)> = Vec::new();
    push_skill_root(&mut found, &grok_home.join("bundled").join("skills"), "内置", 10);
    for root in user_skill_roots(&home) {
        push_skill_root(&mut found, &root, "个人", 20);
    }
    let chain = folder_chain(cwd);
    for (index, dir) in chain.iter().rev().enumerate() {
        let scope = if paths_eq(&dir.display().to_string(), &cwd.display().to_string()) {
            "当前目录"
        } else {
            "仓库"
        };
        let priority = 30 + index as i32;
        for root in project_skill_roots(dir) {
            push_skill_root(&mut found, &root, scope, priority);
        }
    }
    found.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.name.cmp(&right.1.name)));
    let mut best: Vec<(i32, SkillRow)> = Vec::new();
    for (priority, skill) in found {
        if let Some(existing) = best.iter_mut().find(|(_, item)| item.name == skill.name) {
            if priority >= existing.0 {
                *existing = (priority, skill);
            }
        } else {
            best.push((priority, skill));
        }
    }
    best.sort_by(|left, right| left.1.name.cmp(&right.1.name));
    best.into_iter().map(|(_, skill)| skill).take(80).collect()
}

fn user_skill_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".claude").join("skills"),
        home.join(".claude").join("commands"),
        home.join(".cursor").join("skills"),
        home.join(".agents").join("skills"),
        home.join(".agents").join("commands"),
        home.join(".grok").join("skills"),
        home.join(".grok").join("commands"),
    ]
}

fn project_skill_roots(dir: &Path) -> Vec<PathBuf> {
    vec![
        dir.join(".claude").join("skills"),
        dir.join(".claude").join("commands"),
        dir.join(".cursor").join("skills"),
        dir.join(".agents").join("skills"),
        dir.join(".agents").join("commands"),
        dir.join(".grok").join("skills"),
        dir.join(".grok").join("commands"),
    ]
}

fn push_skill_root(found: &mut Vec<(i32, SkillRow)>, root: &Path, scope: &str, priority: i32) {
    if !root.is_dir() {
        return;
    }
    let command_dir = root.file_name().and_then(|name| name.to_str()) == Some("commands");
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && command_dir && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            if let Some(skill) = skill_from_file(&path, scope, None) {
                found.push((priority, skill));
            }
            continue;
        }
        if path.is_dir() {
            collect_skill_dirs(&path, scope, priority, 0, found);
        }
    }
}

fn collect_skill_dirs(dir: &Path, scope: &str, priority: i32, depth: usize, found: &mut Vec<(i32, SkillRow)>) {
    if depth > 3 {
        return;
    }
    let skill_md = dir.join("SKILL.md");
    if skill_md.is_file() {
        if let Some(skill) = skill_from_file(&skill_md, scope, dir.file_name().and_then(|name| name.to_str())) {
            found.push((priority, skill));
        }
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_skill_dirs(&path, scope, priority, depth + 1, found);
        }
    }
}

fn skill_from_file(path: &Path, scope: &str, folder_name: Option<&str>) -> Option<SkillRow> {
    let text = read_prefix(path, 16 * 1024)?;
    let (meta, body) = frontmatter(&text);
    let raw_name = meta.get("name").cloned().unwrap_or_default();
    let fallback = folder_name
        .map(|name| name.to_string())
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.to_string())
        })
        .unwrap_or_default();
    let name = normalize_skill_name(if raw_name.is_empty() { &fallback } else { &raw_name });
    if name.is_empty() {
        return None;
    }
    let description = meta.get("description").cloned().filter(|text| !text.is_empty()).unwrap_or_else(|| first_paragraph(body));
    let invocable = meta.get("user-invocable").map(|value| value != "false").unwrap_or(true);
    let hint = meta.get("argument-hint").cloned().unwrap_or_default();
    Some(SkillRow {
        name,
        description: truncate_chars(&description, 180),
        scope: scope.to_string(),
        invocable,
        hint: truncate_chars(&hint, 80),
        path: path.display().to_string(),
    })
}

pub fn list_hooks(grok_home: &Path, home: &Path, cwd: &Path) -> Vec<HookRow> {
    let mut rows = Vec::new();
    push_hook_dir(&mut rows, &grok_home.join("hooks"), "个人");
    push_hook_file(&mut rows, &home.join(".claude").join("settings.json"), "个人");
    push_hook_file(&mut rows, &home.join(".claude").join("settings.local.json"), "个人");
    push_hook_file(&mut rows, &home.join(".cursor").join("hooks.json"), "个人");
    for dir in folder_chain(cwd) {
        if paths_eq(&dir.display().to_string(), &home.display().to_string()) {
            continue;
        }
        push_hook_dir(&mut rows, &dir.join(".grok").join("hooks"), "项目");
        push_hook_file(&mut rows, &dir.join(".claude").join("settings.json"), "项目");
        push_hook_file(&mut rows, &dir.join(".claude").join("settings.local.json"), "项目");
        push_hook_file(&mut rows, &dir.join(".cursor").join("hooks.json"), "项目");
    }
    rows.truncate(80);
    rows
}

fn push_hook_dir(rows: &mut Vec<HookRow>, dir: &Path, scope: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            push_hook_file(rows, &path, scope);
        }
    }
}

fn push_hook_file(rows: &mut Vec<HookRow>, path: &Path, scope: &str) {
    let Some(text) = read_prefix(path, 200 * 1024) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let Some(hooks) = value.get("hooks").and_then(Value::as_object) else {
        return;
    };
    for (event, groups) in hooks {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            if let Some(nested) = group.get("hooks").and_then(Value::as_array) {
                let matcher = group.get("matcher").and_then(Value::as_str).unwrap_or("").to_string();
                for hook in nested {
                    rows.push(hook_row(scope, event, &matcher, hook, path, true));
                }
            } else if group.get("command").is_some() || group.get("url").is_some() {
                let matcher = group.get("matcher").and_then(Value::as_str).unwrap_or("").to_string();
                rows.push(hook_row(scope, event, &matcher, group, path, false));
            }
        }
    }
}

fn hook_row(scope: &str, event: &str, matcher: &str, hook: &Value, file: &Path, runs: bool) -> HookRow {
    let kind = hook.get("type").and_then(Value::as_str).unwrap_or("command").to_string();
    let target = hook
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| hook.get("url").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    HookRow {
        scope: scope.to_string(),
        event: event.to_string(),
        matcher: matcher.to_string(),
        kind,
        target: truncate_chars(&target, 180),
        runs,
        file: file.display().to_string(),
    }
}

pub fn trust_note(cwd: &Path, trust_text: &str, has_project: bool) -> String {
    let base = "钩子和技能由命令行加载，这里只列出，不代为执行。";
    if is_home(cwd) || trust_contains(trust_text, cwd) {
        return base.to_string();
    }
    if has_project {
        format!("{base}这个文件夹还没信任，项目里的钩子和技能不会加载。需要时在对话里发送 /hooks-trust。")
    } else {
        base.to_string()
    }
}

fn trust_contains(trust_text: &str, cwd: &Path) -> bool {
    if trust_text.is_empty() {
        return false;
    }
    let text = cwd.display().to_string();
    let forward = text.replace('\\', "/");
    trust_text.contains(&text) || trust_text.contains(&forward)
}

pub fn parse_grok_worktrees(text: &str) -> Vec<WorktreeRow> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let path = item.get("path").and_then(Value::as_str).unwrap_or("").trim().to_string();
            if path.is_empty() {
                return None;
            }
            let id = item.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let label = item
                .pointer("/metadata/label")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .unwrap_or(if id.is_empty() { "工作树" } else { &id })
                .to_string();
            let branch = match item.get("git_ref").and_then(Value::as_str).unwrap_or("") {
                "" => String::new(),
                "HEAD" => "当前提交".to_string(),
                other => other.trim_start_matches("refs/heads/").to_string(),
            };
            let status = match item.get("status").and_then(Value::as_str).unwrap_or("") {
                "alive" => "使用中".to_string(),
                "missing" => "目录不在了".to_string(),
                "expired" => "已过期".to_string(),
                "" => "未知".to_string(),
                other => other.to_string(),
            };
            Some(WorktreeRow {
                id,
                path,
                label,
                branch,
                status,
                kind: "grok".to_string(),
                removable: true,
            })
        })
        .collect()
}

pub fn parse_git_worktrees(cwd: &Path, text: &str) -> Vec<WorktreeRow> {
    let mut rows = Vec::new();
    let mut path = String::new();
    let mut branch = String::new();
    let mut detached = false;
    let mut first = true;
    let flush = |rows: &mut Vec<WorktreeRow>, path: &str, branch: &str, detached: bool, first: bool, cwd: &Path| {
        if path.is_empty() {
            return;
        }
        let here = paths_eq(path, &cwd.display().to_string());
        let label = if first || here {
            "当前目录".to_string()
        } else {
            Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("工作树")
                .to_string()
        };
        let branch = if detached {
            "游离".to_string()
        } else {
            branch.trim_start_matches("refs/heads/").to_string()
        };
        rows.push(WorktreeRow {
            id: String::new(),
            path: path.to_string(),
            label,
            branch,
            status: "git".to_string(),
            kind: "git".to_string(),
            removable: !first && !here,
        });
    };
    for line in text.lines() {
        if line.is_empty() {
            flush(&mut rows, &path, &branch, detached, first, cwd);
            path.clear();
            branch.clear();
            detached = false;
            first = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            path = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = rest.trim().to_string();
        } else if line == "detached" {
            detached = true;
        }
    }
    flush(&mut rows, &path, &branch, detached, first, cwd);
    rows
}

pub fn merge_listed(cwd: &Path, grok_rows: Vec<WorktreeRow>, git_rows: Vec<WorktreeRow>) -> Vec<WorktreeRow> {
    let mut rows = Vec::new();
    for row in &git_rows {
        if !row.removable {
            rows.push(row.clone());
        }
    }
    for mut row in grok_rows {
        if paths_eq(&row.path, &cwd.display().to_string()) {
            row.removable = false;
        }
        if !rows.iter().any(|existing| paths_eq(&existing.path, &row.path)) {
            rows.push(row);
        }
    }
    for row in git_rows {
        if row.removable && !rows.iter().any(|existing| paths_eq(&existing.path, &row.path)) {
            rows.push(row);
        }
    }
    rows
}

pub fn list_subagents(grok_home: &Path, cwd: &Path, session_id: &str) -> Vec<SubagentRow> {
    let Some(group) = session_group(&grok_home.join("sessions"), cwd) else {
        return Vec::new();
    };
    let root = group.join(session_id).join("subagents");
    if !root.is_dir() {
        return Vec::new();
    }
    let mut rows = Vec::new();
    walk_subagents(&root, 0, &mut rows);
    rows
}

fn walk_subagents(dir: &Path, depth: usize, rows: &mut Vec<SubagentRow>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_subagents(&path, depth + 1, rows);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            if let Some(row) = read_subagent(&path) {
                rows.push(row);
            }
        }
    }
}

fn read_subagent(path: &Path) -> Option<SubagentRow> {
    let text = read_prefix(path, 64 * 1024)?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let parent = path.parent().and_then(|dir| dir.file_name()).and_then(|name| name.to_str()).unwrap_or("");
    let id = first_text(&value, &["id", "subagent_id", "subagentId"]);
    let id = if id.is_empty() { parent.to_string() } else { id };
    if id.is_empty() {
        return None;
    }
    let title = first_text(&value, &["description", "title", "name"]);
    let title = if title.is_empty() { id.clone() } else { title };
    let kind = first_text(&value, &["subagent_type", "subagentType", "agent_type"]);
    let isolation = first_text(&value, &["isolation"]);
    let status = first_text(&value, &["status"]);
    Some(SubagentRow {
        id,
        title: truncate_chars(&title, 80),
        status,
        kind,
        isolation,
    })
}

pub fn validate_worktree_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 64
        || name.starts_with('.')
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err("名字只能用字母、数字、点、横线，最长 64 个字符。".into());
    }
    Ok(name)
}

pub fn create_worktree(cwd: &Path, name: &str) -> Result<String, String> {
    let name = validate_worktree_name(name)?;
    if !cwd.is_dir() {
        return Err("这个文件夹不存在。".into());
    }
    let grok = crate::cli::find_grok().ok_or("没有找到 grok.exe。")?;
    let output = hidden_command(grok)
        .args(["worktree", "create", name])
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("无法新建工作树：{error}"))?;
    if !output.status.success() {
        return Err(command_error("新建工作树失败。", &output));
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        Ok(name.to_string())
    } else {
        Ok(path)
    }
}

pub fn remove_worktree(cwd: &Path, id: &str, path: &str, kind: &str) -> Result<(), String> {
    let rows = current_worktrees(cwd)?;
    let found = rows.iter().find(|row| {
        row.kind == kind && ((kind == "grok" && !id.is_empty() && row.id == id) || paths_eq(&row.path, path))
    });
    let Some(found) = found else {
        return Err("列表里没有这个工作树。".into());
    };
    if !found.removable {
        return Err("当前目录不能移除。".into());
    }
    if kind == "grok" {
        let grok = crate::cli::find_grok().ok_or("没有找到 grok.exe。")?;
        let output = hidden_command(grok)
            .args(["worktree", "rm", &found.id])
            .current_dir(cwd)
            .output()
            .map_err(|error| format!("无法移除工作树：{error}"))?;
        if !output.status.success() {
            return Err(command_error("移除工作树失败。", &output));
        }
        return Ok(());
    }
    let output = hidden_command(PathBuf::from("git"))
        .args(["worktree", "remove", &found.path])
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("无法移除工作树：{error}"))?;
    if !output.status.success() {
        return Err(command_error("移除工作树失败。", &output));
    }
    Ok(())
}

pub fn current_worktrees(cwd: &Path) -> Result<Vec<WorktreeRow>, String> {
    let (json, porcelain, note) = worktree_sources(cwd);
    if json.is_none() && porcelain.is_none() {
        return Err(if note.is_empty() {
            "没有读到工作树。".into()
        } else {
            note
        });
    }
    Ok(merge_listed(
        cwd,
        json.as_deref().map(parse_grok_worktrees).unwrap_or_default(),
        porcelain.as_deref().map(|text| parse_git_worktrees(cwd, text)).unwrap_or_default(),
    ))
}

pub fn worktree_sources(cwd: &Path) -> (Option<String>, Option<String>, String) {
    let json = crate::cli::find_grok().and_then(|grok| {
        hidden_command(grok)
            .args(["worktree", "list", "--json"])
            .current_dir(cwd)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
    });
    let porcelain = hidden_command(PathBuf::from("git"))
        .args(["worktree", "list", "--porcelain"])
        .current_dir(cwd)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string());
    let note = if json.is_none() && porcelain.is_none() {
        "这个文件夹没有工作树。它可能不是 git 仓库。".to_string()
    } else {
        String::new()
    };
    (json, porcelain, note)
}

pub fn read_recent(file: &Path) -> Vec<String> {
    let Ok(text) = fs::read_to_string(file) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

pub fn push_recent(file: &Path, cwd: &str) -> Vec<String> {
    let mut rows = read_recent(file);
    rows.retain(|item| !paths_eq(item, cwd));
    rows.insert(0, cwd.to_string());
    rows.truncate(8);
    if let Some(parent) = file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&rows) {
        let _ = fs::write(file, text);
    }
    rows
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveMark {
    id: String,
    cwd: String,
    at: String,
}

pub(crate) fn read_archived(file: &Path) -> Vec<ArchiveMark> {
    let Ok(text) = fs::read_to_string(file) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn set_archived(file: &Path, cwd: &str, id: &str, archived: bool) -> Result<(), String> {
    if !valid_session_id(id) {
        return Err("会话编号不对。".into());
    }
    let mut rows = read_archived(file);
    rows.retain(|row| !(row.id == id && paths_eq(&row.cwd, cwd)));
    if archived {
        rows.insert(
            0,
            ArchiveMark {
                id: id.to_string(),
                cwd: cwd.to_string(),
                at: unix_now(),
            },
        );
    }
    rows.truncate(500);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(|_| "写不了归档记录。".to_string())?;
    }
    let text = serde_json::to_string_pretty(&rows).map_err(|_| "写不了归档记录。".to_string())?;
    fs::write(file, text).map_err(|_| "写不了归档记录。".to_string())?;
    Ok(())
}

fn apply_archive_flags(sessions: &mut [SessionRow], cwd: &str, marks: &[ArchiveMark]) {
    for row in sessions {
        row.archived = marks.iter().any(|mark| mark.id == row.id && paths_eq(&mark.cwd, cwd));
    }
}

pub fn valid_session_id(id: &str) -> bool {
    let len = id.len();
    (8..=80).contains(&len) && id.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

pub fn session_directory(grok_home: &Path, cwd: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_session_id(id) {
        return Err("会话编号不对。".into());
    }
    let group = session_group(&grok_home.join("sessions"), cwd).ok_or("这个文件夹里没有这个会话。")?;
    let direct = group.join(id);
    if direct.is_dir() && is_direct_child(&group, &direct) {
        return Ok(direct);
    }
    if let Ok(entries) = fs::read_dir(&group) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && is_direct_child(&group, &path) {
                if read_summary(&path).is_some_and(|row| row.id == id) {
                    return Ok(path);
                }
            }
        }
    }
    Err("这个文件夹里没有这个会话。".into())
}

fn is_direct_child(group: &Path, dir: &Path) -> bool {
    dir.parent()
        .is_some_and(|parent| paths_eq(&parent.display().to_string(), &group.display().to_string()))
}

pub fn delete_session_cli(cwd: &Path, id: &str) -> Result<(), String> {
    let dir = session_directory(&grok_home(), cwd, id)?;
    let grok = crate::cli::find_grok().ok_or_else(|| "没有找到 grok.exe。".to_string())?;
    let output = hidden_command(&grok)
        .args(["sessions", "delete", id])
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("无法删除会话：{error}"))?;
    if !dir.exists() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.contains("No session found") {
        return Err("命令行没有找到这个会话，文件还在。".into());
    }
    Err(command_error("命令行没有删掉这个会话。", &output))
}

const OUTPUT_CAP: usize = 40;
const LINE_CAP: usize = 512 * 1024;
const PREVIEW_CAP: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputFile {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOutputs {
    pub files: Vec<OutputFile>,
    pub links: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub path: String,
    pub name: String,
    pub text: String,
    pub binary: bool,
    pub truncated: bool,
}

#[cfg(test)]
pub fn outputs_from_jsonl(text: &str) -> SessionOutputs {
    let mut files = Vec::new();
    let mut links = Vec::new();
    for line in text.lines() {
        absorb_line(line, false, &mut files, &mut links);
    }
    SessionOutputs { files, links }
}

pub fn outputs_from_file(path: &Path) -> SessionOutputs {
    let mut files = Vec::new();
    let mut links = Vec::new();
    let Ok(file) = fs::File::open(path) else {
        return SessionOutputs { files, links };
    };
    let mut reader = std::io::BufReader::new(file);
    while let Some((line, cut)) = next_capped_line(&mut reader, LINE_CAP) {
        absorb_line(&line, cut, &mut files, &mut links);
        if files.len() >= OUTPUT_CAP && links.len() >= OUTPUT_CAP {
            break;
        }
    }
    SessionOutputs { files, links }
}

fn next_capped_line(reader: &mut impl BufRead, cap: usize) -> Option<(String, bool)> {
    let mut buf = Vec::new();
    let read = reader.take((cap as u64) + 1).read_until(b'\n', &mut buf).ok()?;
    if read == 0 {
        return None;
    }
    let ended = buf.last() == Some(&b'\n');
    let cut = buf.len() > cap && !ended;
    if cut {
        buf.truncate(cap);
        let mut discard = Vec::new();
        let _ = reader.read_until(b'\n', &mut discard);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Some((String::from_utf8_lossy(&buf).into_owned(), cut))
}

fn absorb_line(line: &str, cut: bool, files: &mut Vec<OutputFile>, links: &mut Vec<String>) {
    if cut {
        loose_scan_diff(line, files);
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let Some(update) = value.pointer("/params/update") else {
        return;
    };
    let kind = update.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    match kind {
        "agent_message_chunk" => {
            if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                push_urls(text, links);
            }
        }
        "tool_call" | "tool_call_update" => {
            let Some(items) = update.get("content").and_then(Value::as_array) else {
                return;
            };
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("diff") {
                    if let Some(path) = item.get("path").and_then(Value::as_str) {
                        push_file(path, files);
                    }
                }
                if let Some(text) = item
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("text").and_then(Value::as_str))
                {
                    push_urls(text, links);
                }
            }
        }
        _ => {}
    }
}

fn loose_scan_diff(text: &str, files: &mut Vec<OutputFile>) {
    let mut rest = text;
    while let Some(index) = rest.find("\"diff\"") {
        let after = index + 6;
        let window_end = rest.len().min(after + 500);
        if let Some(path) = quoted_after_key(&rest[after..window_end], "path") {
            push_file(&path, files);
        }
        rest = &rest[after..];
    }
}

fn quoted_after_key(window: &str, key: &str) -> Option<String> {
    let pattern = format!("\"{key}\"");
    let index = window.find(&pattern)?;
    let after = &window[index + pattern.len()..];
    let colon = after.find(':')?;
    let after = after[colon + 1..].trim_start();
    after.strip_prefix('"').and_then(unescape_json_string)
}

fn unescape_json_string(input: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            return Some(out);
        }
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next()? {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            '/' => out.push('/'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let hex: String = chars.by_ref().take(4).collect();
                let code = u32::from_str_radix(&hex, 16).ok()?;
                out.push(char::from_u32(code)?);
            }
            other => out.push(other),
        }
    }
    None
}

fn push_file(path: &str, files: &mut Vec<OutputFile>) {
    let path = path.trim();
    if path.is_empty() || path == "文件" || files.len() >= OUTPUT_CAP {
        return;
    }
    if path.chars().any(|ch| ch.is_control()) {
        return;
    }
    if files.iter().any(|file| paths_eq(&file.path, path)) {
        return;
    }
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string();
    files.push(OutputFile {
        path: path.to_string(),
        name,
    });
}

fn push_urls(text: &str, links: &mut Vec<String>) {
    for url in extract_urls(text) {
        if links.len() >= OUTPUT_CAP {
            break;
        }
        if !links.iter().any(|item| item == &url) {
            links.push(url);
        }
    }
}

pub fn extract_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let lower = rest.as_bytes();
        let scheme = if starts_with_ignore_ascii(lower, b"https://") {
            Some(8)
        } else if starts_with_ignore_ascii(lower, b"http://") {
            Some(7)
        } else {
            None
        };
        if let Some(scheme_len) = scheme {
            let start = index;
            let mut end = index + scheme_len;
            while end < text.len() {
                let ch = text[end..].chars().next().unwrap();
                if ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '"' | '\'' | '`' | '\\') {
                    break;
                }
                end += ch.len_utf8();
            }
            while end > start + scheme_len {
                let last = text[..end].chars().next_back().unwrap();
                if matches!(last, ')' | ']' | '}' | '.' | ',' | ';' | '!' | '?' | ':') {
                    end -= last.len_utf8();
                } else {
                    break;
                }
            }
            let url = &text[start..end];
            if is_http_url(url) && !out.iter().any(|item| item == url) {
                out.push(url.to_string());
            }
            index = end.max(start + 1);
        } else {
            index += text[index..].chars().next().unwrap().len_utf8();
        }
    }
    out
}

fn starts_with_ignore_ascii(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.len() >= prefix.len()
        && bytes[..prefix.len()].iter().zip(prefix).all(|(left, right)| left.to_ascii_lowercase() == *right)
}

pub fn is_http_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = if let Some(rest) = lower.strip_prefix("https://") {
        rest
    } else if let Some(rest) = lower.strip_prefix("http://") {
        rest
    } else {
        return false;
    };
    if rest.is_empty() || url.len() > 2000 {
        return false;
    }
    !url.chars()
        .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '<' | '>' | '"' | '\'' | '`' | '\\'))
}

pub fn preview_existing_file(path: &str) -> Result<FilePreview, String> {
    let path = checked_path(path, true)?;
    let mut file = fs::File::open(&path).map_err(|_| "文件不在了。".to_string())?;
    let mut buf = vec![0u8; PREVIEW_CAP + 1];
    let read = file.read(&mut buf).map_err(|_| "读不了这个文件。".to_string())?;
    buf.truncate(read);
    let truncated = buf.len() > PREVIEW_CAP;
    if truncated {
        buf.truncate(PREVIEW_CAP);
    }
    let name = file_label(&path);
    if buf.contains(&0) {
        return Ok(FilePreview {
            path: path.display().to_string(),
            name,
            text: String::new(),
            binary: true,
            truncated: false,
        });
    }
    Ok(FilePreview {
        path: path.display().to_string(),
        name,
        text: String::from_utf8_lossy(&buf).into_owned(),
        binary: false,
        truncated,
    })
}

pub fn open_http(url: &str) -> Result<(), String> {
    let url = url.trim();
    if !is_http_url(url) {
        return Err("只能打开 http 或 https 链接。".into());
    }
    open_with_system(url)
}

pub fn open_existing_path(path: &str) -> Result<(), String> {
    let path = checked_path(path, false)?;
    open_with_system(&path.display().to_string())
}

fn checked_path(path: &str, file_only: bool) -> Result<PathBuf, String> {
    let path = path.trim();
    if path.is_empty() || path.chars().any(|ch| ch.is_control()) {
        return Err("这个路径打不开。".into());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() || path.components().any(|component| matches!(component, Component::ParentDir)) {
        return Err("这个路径打不开。".into());
    }
    let exists = if file_only { path.is_file() } else { path.exists() };
    if !exists {
        return Err("文件不在了。".into());
    }
    Ok(path)
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("文件")
        .to_string()
}

fn unix_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(windows)]
fn open_with_system(target: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
    }
    let operation = wide("open");
    let file = wide(target);
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        ) as isize
    };
    if code > 32 {
        Ok(())
    } else {
        Err("系统没有打开它。".into())
    }
}

#[cfg(windows)]
#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: *mut core::ffi::c_void,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show_cmd: i32,
    ) -> *mut core::ffi::c_void;
}

#[cfg(not(windows))]
fn open_with_system(target: &str) -> Result<(), String> {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let status = Command::new(program)
        .arg(target)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| format!("系统没有打开它：{error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("系统没有打开它。".into())
    }
}

pub fn user_home() -> PathBuf {
    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(".")
}

pub fn grok_home() -> PathBuf {
    if let Ok(home) = std::env::var("GROK_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    user_home().join(".grok")
}

fn is_home(cwd: &Path) -> bool {
    paths_eq(&cwd.display().to_string(), &user_home().display().to_string())
}

fn folder_chain(cwd: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let mut current = Some(cwd.to_path_buf());
    for _ in 0..16 {
        let Some(dir) = current else {
            break;
        };
        let root = dir.join(".git").exists();
        let parent = dir.parent().map(|path| path.to_path_buf());
        chain.push(dir);
        if root {
            break;
        }
        current = parent;
    }
    chain
}

pub fn paths_eq(left: &str, right: &str) -> bool {
    let left = normalize_path(left);
    let right = normalize_path(right);
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(&right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn normalize_path(path: &str) -> String {
    path.trim().trim_end_matches(['\\', '/']).replace('\\', "/")
}

fn text_at(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        current = match current.get(*key) {
            Some(next) => next,
            None => return String::new(),
        };
    }
    current.as_str().unwrap_or("").trim().to_string()
}

fn first_text(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let text = text.trim();
            if !text.is_empty() {
                return text.to_string();
            }
        }
    }
    String::new()
}

fn read_prefix(path: &Path, max_bytes: usize) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let end = bytes.len().min(max_bytes);
    Some(String::from_utf8_lossy(&bytes[..end]).to_string())
}

fn frontmatter(text: &str) -> (std::collections::BTreeMap<String, String>, &str) {
    let mut map = std::collections::BTreeMap::new();
    let stripped = text.trim_start_matches('\u{feff}');
    let Some(rest) = stripped.strip_prefix("---") else {
        return (map, text);
    };
    let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')).unwrap_or(rest);
    let end = rest.find("\r\n---").or_else(|| rest.find("\n---"));
    let Some(end) = end else {
        return (map, text);
    };
    let yaml = &rest[..end];
    let body = &rest[end..];
    let mut key: Option<String> = None;
    let mut folded = false;
    let mut value = String::new();
    let flush = |map: &mut std::collections::BTreeMap<String, String>, key: &mut Option<String>, value: &mut String, folded: bool| {
        if let Some(name) = key.take() {
            let text = if folded {
                value.split_whitespace().collect::<Vec<_>>().join(" ")
            } else {
                value.trim().to_string()
            };
            map.insert(name, text);
            value.clear();
        }
    };
    for line in yaml.lines() {
        let continued = line.starts_with(' ') || line.starts_with('\t');
        if continued && key.is_some() {
            value.push('\n');
            value.push_str(line.trim());
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once(':') else {
            continue;
        };
        if raw_key.is_empty() || raw_key.starts_with(' ') {
            continue;
        }
        flush(&mut map, &mut key, &mut value, folded);
        let raw_value = raw_value.trim().trim_matches('"').trim_matches('\'').trim();
        folded = matches!(raw_value, ">" | ">-" | "|" | "|-");
        key = Some(raw_key.trim().to_string());
        if !folded {
            value = raw_value.to_string();
            flush(&mut map, &mut key, &mut value, false);
        }
    }
    flush(&mut map, &mut key, &mut value, folded);
    (map, body)
}

fn first_paragraph(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("")
        .to_string()
}

fn normalize_skill_name(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in name.trim().chars() {
        let ch = if ch == '_' || ch.is_whitespace() { '-' } else { ch };
        if ch == '-' {
            if !dash && !out.is_empty() {
                out.push('-');
            }
            dash = true;
            continue;
        }
        dash = false;
        out.push(ch.to_ascii_lowercase());
    }
    out.trim_matches('-').to_string()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.trim().to_string();
    }
    let cut: String = text.trim().chars().take(max_chars).collect();
    format!("{cut}…")
}

fn hidden_command(program: impl AsRef<Path>) -> Command {
    let mut command = Command::new(program.as_ref());
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn command_error(title: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if !stderr.trim().is_empty() { stderr.trim() } else { stdout.trim() };
    if detail.is_empty() {
        title.to_string()
    } else {
        format!("{title}\n{}", truncate_chars(detail, 400))
    }
}

#[tauri::command]
pub fn project_catalog(app: tauri::AppHandle, cwd: String, session_id: String) -> Result<Catalog, String> {
    let cwd_path = PathBuf::from(cwd.trim());
    if !cwd_path.is_dir() {
        return Err("这个文件夹不存在。".into());
    }
    let home = grok_home();
    let trust = fs::read_to_string(home.join("trusted_folders.toml")).unwrap_or_default();
    let (json, porcelain, note) = worktree_sources(&cwd_path);
    let mut catalog = build_catalog(&CatalogInput {
        grok_home: &home,
        cwd: &cwd_path,
        session_id: session_id.trim(),
        trust_text: &trust,
        worktree_json: json.as_deref(),
        git_porcelain: porcelain.as_deref(),
    });
    catalog.worktree_note = note;
    if let Ok(dir) = app.path().app_config_dir() {
        catalog.recent = read_recent(&dir.join("recent-projects.json"));
        let marks = read_archived(&dir.join("archived-sessions.json"));
        apply_archive_flags(&mut catalog.sessions, cwd.trim(), &marks);
    }
    Ok(catalog)
}

#[tauri::command]
pub fn set_session_archived(
    app: tauri::AppHandle,
    cwd: String,
    session_id: String,
    archived: bool,
) -> Result<(), String> {
    let cwd = cwd.trim();
    let id = session_id.trim();
    if !Path::new(cwd).is_dir() {
        return Err("这个文件夹不存在。".into());
    }
    if archived {
        session_directory(&grok_home(), Path::new(cwd), id)?;
    } else if !valid_session_id(id) {
        return Err("会话编号不对。".into());
    }
    let dir = app.path().app_config_dir().map_err(|_| "写不了归档记录。".to_string())?;
    set_archived(&dir.join("archived-sessions.json"), cwd, id, archived)
}

#[tauri::command]
pub fn session_outputs(cwd: String, session_id: String) -> Result<SessionOutputs, String> {
    let cwd_path = PathBuf::from(cwd.trim());
    if !cwd_path.is_dir() {
        return Err("这个文件夹不存在。".into());
    }
    let dir = session_directory(&grok_home(), &cwd_path, session_id.trim())?;
    Ok(outputs_from_file(&dir.join("updates.jsonl")))
}

#[tauri::command]
pub fn preview_file(path: String) -> Result<FilePreview, String> {
    preview_existing_file(&path)
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    open_http(&url)
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    open_existing_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_windows_path_the_way_grok_names_the_group() {
        assert_eq!(encode_cwd(r"C:\Users\me"), "C%3A%5CUsers%5Cme");
        assert_eq!(encode_cwd(r"e:\My Projects"), "e%3A%5CMy%20Projects");
    }

    #[test]
    fn lists_and_filters_sessions_from_summary_files() {
        let root = std::env::temp_dir().join(format!("grok-desk-sessions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let group = root.join(".grok").join("sessions").join(encode_cwd(&cwd.display().to_string()));
        let older = group.join("old");
        let newer = group.join("new");
        fs::create_dir_all(&older).unwrap();
        fs::create_dir_all(&newer).unwrap();
        fs::write(
            older.join("summary.json"),
            r#"{"info":{"id":"old"},"generated_title":"旧的","session_summary":"以前","updated_at":"2026-01-01T00:00:00Z","current_model_id":"grok-4.5"}"#,
        )
        .unwrap();
        fs::write(
            newer.join("summary.json"),
            r#"{"info":{"id":"new"},"generated_title":"修登录","session_summary":"登录失败","updated_at":"2026-02-01T00:00:00Z","current_model_id":"grok-4.7"}"#,
        )
        .unwrap();
        let rows = list_sessions(&root.join(".grok"), &cwd);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "new");
        assert_eq!(rows[0].title, "修登录");
        let hashed = root.join(".grok").join("sessions").join("long-name");
        fs::create_dir_all(hashed.join("only")).unwrap();
        fs::write(hashed.join(".cwd"), cwd.display().to_string()).unwrap();
        fs::write(
            hashed.join("only").join("summary.json"),
            r#"{"info":{"id":"only"},"session_summary":"哈希目录","updated_at":"2026-03-01T00:00:00Z"}"#,
        )
        .unwrap();
        let _ = fs::remove_dir_all(&group);
        let rows = list_sessions(&root.join(".grok"), &cwd);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "哈希目录");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn project_skill_overrides_the_personal_one() {
        let root = std::env::temp_dir().join(format!("grok-desk-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("repo");
        fs::create_dir_all(home.join(".grok").join("skills").join("review")).unwrap();
        fs::create_dir_all(project.join(".grok").join("skills").join("review")).unwrap();
        fs::create_dir_all(project.join(".grok").join("skills").join("hidden")).unwrap();
        fs::write(
            home.join(".grok").join("skills").join("review").join("SKILL.md"),
            "---\nname: review\ndescription: 个人说明\n---\n# 个人\n",
        )
        .unwrap();
        fs::write(
            project.join(".grok").join("skills").join("review").join("SKILL.md"),
            "---\nname: review\ndescription: >-\n  项目说明\n  第二行\nuser-invocable: false\nargument-hint: 文件\n---\n",
        )
        .unwrap();
        fs::write(
            project.join(".grok").join("skills").join("hidden").join("SKILL.md"),
            "# 没有名字时用目录\n\n先看测试。\n",
        )
        .unwrap();
        let skills = list_skills(&home.join(".grok"), &home, &project);
        let review = skills.iter().find(|skill| skill.name == "review").unwrap();
        assert_eq!(skills.iter().filter(|skill| skill.name == "review").count(), 1);
        assert_eq!(review.scope, "当前目录");
        assert_eq!(review.description, "项目说明 第二行");
        assert!(!review.invocable);
        assert_eq!(review.hint, "文件");
        assert!(skills.iter().any(|skill| skill.name == "hidden" && skill.description.contains("先看测试")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_hook_files_and_marks_cursor_flat_entries() {
        let root = std::env::temp_dir().join(format!("grok-desk-hooks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let project = root.join("repo");
        fs::create_dir_all(home.join(".grok").join("hooks")).unwrap();
        fs::create_dir_all(project.join(".cursor")).unwrap();
        fs::write(
            home.join(".grok").join("hooks").join("start.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo hi"}]}]}}"#,
        )
        .unwrap();
        fs::write(
            project.join(".cursor").join("hooks.json"),
            r#"{"hooks":{"preToolUse":[{"command":"nope","matcher":"Bash"}]}}"#,
        )
        .unwrap();
        let hooks = list_hooks(&home.join(".grok"), &home, &project);
        assert!(hooks.iter().any(|hook| hook.event == "SessionStart" && hook.runs && hook.scope == "个人"));
        assert!(hooks.iter().any(|hook| hook.event == "preToolUse" && !hook.runs && hook.scope == "项目"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merges_grok_worktrees_with_the_current_git_directory() {
        let json = r#"[{"id":"probe-1","path":"C:\\wt\\probe","git_ref":"HEAD","status":"alive","metadata":{"label":"probe"}}]"#;
        let porcelain = "worktree C:/repo\nHEAD abc\nbranch refs/heads/main\n\nworktree C:/wt/other\nHEAD def\ndetached\n";
        let cwd = Path::new(r"C:\repo");
        let rows = merge_listed(cwd, parse_grok_worktrees(json), parse_git_worktrees(cwd, porcelain));
        assert_eq!(rows[0].label, "当前目录");
        assert!(!rows[0].removable);
        let grok = rows.iter().find(|row| row.id == "probe-1").unwrap();
        assert_eq!(grok.label, "probe");
        assert_eq!(grok.branch, "当前提交");
        assert_eq!(grok.status, "使用中");
        assert!(grok.removable);
        assert!(rows.iter().any(|row| row.path == "C:/wt/other" && row.removable && row.branch == "游离"));
    }

    #[test]
    fn rejects_a_worktree_name_with_a_path() {
        assert!(validate_worktree_name("fix-login").is_ok());
        assert!(validate_worktree_name("../x").is_err());
        assert!(validate_worktree_name("a b").is_err());
    }

    #[test]
    fn reads_subagent_metadata() {
        let root = std::env::temp_dir().join(format!("grok-desk-sub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let cwd = root.join("work");
        let meta = root
            .join(".grok")
            .join("sessions")
            .join(encode_cwd(&cwd.display().to_string()))
            .join("sess")
            .join("subagents")
            .join("child")
            .join("meta.json");
        fs::create_dir_all(meta.parent().unwrap()).unwrap();
        fs::write(
            &meta,
            r#"{"id":"child","description":"看结构","subagent_type":"explore","isolation":"worktree","status":"completed"}"#,
        )
        .unwrap();
        let rows = list_subagents(&root.join(".grok"), &cwd, "sess");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "看结构");
        assert_eq!(rows[0].kind, "explore");
        assert_eq!(rows[0].isolation, "worktree");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recent_projects_keep_the_newest_first() {
        let file = std::env::temp_dir().join(format!("grok-desk-recent-{}.json", std::process::id()));
        let _ = fs::remove_file(&file);
        push_recent(&file, r"C:\one");
        let rows = push_recent(&file, r"C:\two");
        assert_eq!(rows, vec![r"C:\two".to_string(), r"C:\one".to_string()]);
        let rows = push_recent(&file, r"c:\two");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], r"c:\two");
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn archive_marks_round_trip_without_touching_a_session() {
        let file = std::env::temp_dir().join(format!("grok-desk-arch-{}.json", std::process::id()));
        let _ = fs::remove_file(&file);
        let id = "01a17375-17a4-70d3-80ed-a827c0173780";
        set_archived(&file, r"C:\work", id, true).unwrap();
        let rows = read_archived(&file);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        set_archived(&file, r"c:\work", id, true).unwrap();
        assert_eq!(read_archived(&file).len(), 1);
        set_archived(&file, r"c:\work", id, false).unwrap();
        assert!(read_archived(&file).is_empty());
        assert!(set_archived(&file, r"C:\work", "../nope", true).is_err());
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn session_directory_requires_a_real_child() {
        let root = std::env::temp_dir().join(format!("grok-desk-sessdir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let cwd = root.join("work");
        fs::create_dir_all(&cwd).unwrap();
        let id = "abcdef12-3456-7890-abcd-ef1234567890";
        let dir = root
            .join(".grok")
            .join("sessions")
            .join(encode_cwd(&cwd.display().to_string()))
            .join(id);
        fs::create_dir_all(&dir).unwrap();
        let found = session_directory(&root.join(".grok"), &cwd, id).unwrap();
        assert!(found.ends_with(id));
        assert!(session_directory(&root.join(".grok"), &cwd, "../secret").is_err());
        assert!(session_directory(&root.join(".grok"), &cwd, "short").is_err());
        assert!(session_directory(&root.join(".grok"), &cwd, "deadbeef-dead-beef-dead-beefdeadbeef").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn collects_diff_paths_and_message_links() {
        let diff = r#"{"method":"session/update","params":{"update":{"sessionUpdate":"tool_call_update","content":[{"type":"diff","path":"C:\\projects\\app\\package.json"},{"type":"content","content":{"text":"see https://example.com/docs."}}]}}}"#;
        let message = r#"{"method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"打开 https://example.com/a 和 javascript:alert(1)"}}}}"#;
        let thought = r#"{"method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"https://secret.example/hidden"}}}}"#;
        let outputs = outputs_from_jsonl(&format!("{diff}\n{message}\n{thought}\n"));
        assert_eq!(outputs.files.len(), 1);
        assert_eq!(outputs.files[0].path, r"C:\projects\app\package.json");
        assert_eq!(outputs.files[0].name, "package.json");
        assert!(outputs.links.iter().any(|link| link == "https://example.com/docs"));
        assert!(outputs.links.iter().any(|link| link == "https://example.com/a"));
        assert!(!outputs.links.iter().any(|link| link.contains("javascript")));
        assert!(!outputs.links.iter().any(|link| link.contains("secret")));
    }

    #[test]
    fn reads_outputs_from_a_session_file() {
        let root = std::env::temp_dir().join(format!("grok-desk-outputs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("updates.jsonl"),
            "{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"tool_call\",\"content\":[{\"type\":\"diff\",\"path\":\"C:\\\\repo\\\\a.txt\"}]}}}\n",
        )
        .unwrap();
        let outputs = outputs_from_file(&root.join("updates.jsonl"));
        assert_eq!(outputs.files[0].path, r"C:\repo\a.txt");
        assert!(outputs_from_file(&root.join("missing.jsonl")).files.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn extracts_http_urls_and_rejects_other_schemes() {
        let urls = extract_urls("看 https://example.com/a. 和 http://ex.test/b) 以及 file:///c/x javascript:alert(1)");
        assert_eq!(
            urls,
            vec!["https://example.com/a".to_string(), "http://ex.test/b".to_string()]
        );
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("file:///c/x"));
        assert!(!is_http_url("https://bad host"));
        assert!(is_http_url("https://example.com/a"));
        assert!(open_http("javascript:alert(1)").is_err());
        assert!(open_http("file:///C:/Windows/notepad.exe").is_err());
        assert!(open_existing_path(r"C:\no\such\file.txt").is_err());
        assert!(open_existing_path(r"C:\Users\..\Windows\notepad.exe").is_err());
    }

    #[test]
    fn preview_reads_text_rejects_parent_and_marks_binary() {
        let root = std::env::temp_dir().join(format!("grok-desk-preview-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let file = root.join("note.txt");
        fs::write(&file, "你好").unwrap();
        let preview = preview_existing_file(&file.display().to_string()).unwrap();
        assert_eq!(preview.text, "你好");
        assert!(!preview.binary);
        let bin = root.join("bin.dat");
        fs::write(&bin, b"a\0b").unwrap();
        let preview = preview_existing_file(&bin.display().to_string()).unwrap();
        assert!(preview.binary);
        assert!(preview.text.is_empty());
        let big = root.join("big.txt");
        fs::write(&big, "x".repeat(33 * 1024)).unwrap();
        let preview = preview_existing_file(&big.display().to_string()).unwrap();
        assert!(preview.truncated);
        assert_eq!(preview.text.len(), 32 * 1024);
        assert!(preview_existing_file(r"C:\missing\nope.txt").is_err());
        let jumped = format!(r"{}\..\note.txt", root.display());
        assert!(preview_existing_file(&jumped).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
