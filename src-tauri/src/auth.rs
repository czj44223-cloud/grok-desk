use std::io::Read;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub logged_in: bool,
    pub name: String,
    pub email: String,
    pub mode: String,
}

impl AuthStatus {
    fn logged_out() -> Self {
        Self {
            logged_in: false,
            name: String::new(),
            email: String::new(),
            mode: String::new(),
        }
    }
}

pub fn parse_auth(text: &str) -> AuthStatus {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return AuthStatus::logged_out();
    };
    let Some(record) = find_record(&value) else {
        return AuthStatus::logged_out();
    };
    let email = field(record, "email");
    let key = field(record, "key");
    let refresh = field(record, "refresh_token");
    let logged_in = !email.is_empty() || !key.is_empty() || !refresh.is_empty();
    if !logged_in {
        return AuthStatus::logged_out();
    }
    let mut name = [field(record, "first_name"), field(record, "last_name")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if name.is_empty() {
        name = email.clone();
    }
    if name.is_empty() {
        name = "已登录".to_string();
    }
    AuthStatus {
        logged_in: true,
        name,
        email,
        mode: field(record, "auth_mode"),
    }
}

fn find_record(value: &Value) -> Option<&Value> {
    if value.get("email").is_some() || value.get("key").is_some() || value.get("refresh_token").is_some() {
        return Some(value);
    }
    let object = value.as_object()?;
    for child in object.values() {
        if let Some(found) = find_record(child) {
            return Some(found);
        }
    }
    None
}

fn field(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string()
}

pub fn safe_login_url(url: &str) -> bool {
    if !url.to_ascii_lowercase().starts_with("https://") || !crate::catalog::is_http_url(url) {
        return false;
    }
    if url.len() > 500 {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    let Some((_, query)) = lower.split_once('?') else {
        return !lower.contains("token");
    };
    for part in query.split(['&', '#']) {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if key == "code" || key.contains("token") || key.contains("secret") || key.contains("password") {
            return false;
        }
        if value.len() > 100 {
            return false;
        }
    }
    true
}

pub fn extract_login_urls(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let https = starts_with_ignore_ascii(rest.as_bytes(), b"https://");
        let http = starts_with_ignore_ascii(rest.as_bytes(), b"http://");
        if https || http {
            let mut end = 0;
            for (offset, ch) in rest.char_indices() {
                if ch.is_whitespace()
                    || ch.is_control()
                    || matches!(ch, '<' | '>' | '"' | '\'' | '`' | '\\' | ')' | ']' | '，' | '。')
                {
                    break;
                }
                end = offset + ch.len_utf8();
            }
            if end > 0 {
                let url = rest[..end].trim_end_matches('.').to_string();
                if !out.iter().any(|item| item == &url) {
                    out.push(url);
                }
                index += end.max(1);
                continue;
            }
        }
        index += text[index..].chars().next().map(|ch| ch.len_utf8()).unwrap_or(1);
    }
    out
}

fn starts_with_ignore_ascii(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.len() >= prefix.len()
        && bytes[..prefix.len()]
            .iter()
            .zip(prefix)
            .all(|(left, right)| left.to_ascii_lowercase() == *right)
}

#[tauri::command]
pub fn auth_status() -> Result<AuthStatus, String> {
    let path = crate::catalog::grok_home().join("auth.json");
    if !path.is_file() {
        return Ok(AuthStatus::logged_out());
    }
    let meta = std::fs::metadata(&path).map_err(|_| "读不了登录状态。".to_string())?;
    if meta.len() > 1_000_000 {
        return Ok(AuthStatus::logged_out());
    }
    let text = std::fs::read_to_string(&path).map_err(|_| "读不了登录状态。".to_string())?;
    Ok(parse_auth(&text))
}

#[tauri::command]
pub fn auth_login() -> Result<String, String> {
    let path = crate::cli::find_grok().ok_or("没有找到 grok.exe。".to_string())?;
    let mut command = std::process::Command::new(path);
    command
        .args(["login", "--oauth"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|_| "无法启动登录。".to_string())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel();
    if let Some(pipe) = stdout {
        let tx = tx.clone();
        std::thread::spawn(move || watch_pipe(pipe, tx));
    }
    if let Some(pipe) = stderr {
        std::thread::spawn(move || watch_pipe(pipe, tx));
    }
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut opened = false;
    loop {
        opened = open_pending(&rx, opened);
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = open_pending(&rx, opened);
                if status.success() && auth_status().map(|status| status.logged_in).unwrap_or(false) {
                    return Ok("已登录。".into());
                }
                return Err("登录没有完成。".into());
            }
            Ok(None) if Instant::now() >= deadline => {
                crate::cli::kill_tree(&mut child);
                return Err("登录没有完成。".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(80)),
            Err(_) => {
                crate::cli::kill_tree(&mut child);
                return Err("登录没有完成。".into());
            }
        }
    }
}

fn open_pending(rx: &mpsc::Receiver<String>, opened: bool) -> bool {
    let mut opened = opened;
    while let Ok(url) = rx.try_recv() {
        if !opened && safe_login_url(&url) {
            opened = true;
            let _ = crate::catalog::open_http(&url);
        }
    }
    opened
}

fn watch_pipe(mut pipe: impl Read, tx: Sender<String>) {
    let mut pending = String::new();
    let mut buf = [0u8; 1024];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                if pending.len() > 8000 {
                    pending = pending.split_off(pending.len() - 4000);
                }
                for url in extract_login_urls(&pending) {
                    let _ = tx.send(url);
                }
            }
        }
    }
}

#[tauri::command]
pub fn auth_logout() -> Result<String, String> {
    let captured = crate::cli::run_grok(&["logout"], Duration::from_secs(30))?;
    if captured.code == 0 && !auth_status().map(|status| status.logged_in).unwrap_or(true) {
        return Ok("已退出。".into());
    }
    Err("退出没有完成。".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_uses_the_display_name_and_drops_secrets() {
        let text = r#"{
            "https://example.test::client": {
                "key": "secret-token-value",
                "refresh_token": "refresh-secret-value",
                "auth_mode": "oidc",
                "email": "person@example.com",
                "first_name": "Ada",
                "last_name": "Lovelace",
                "user_id": "user-should-not-leak",
                "team_id": "team-should-not-leak"
            }
        }"#;
        let status = parse_auth(text);
        assert!(status.logged_in);
        assert_eq!(status.name, "Ada Lovelace");
        assert_eq!(status.email, "person@example.com");
        assert_eq!(status.mode, "oidc");
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("secret-token-value"));
        assert!(!json.contains("refresh-secret-value"));
        assert!(!json.contains("user-should-not-leak"));
        assert!(!json.contains("team-should-not-leak"));
    }

    #[test]
    fn empty_auth_is_logged_out() {
        let status = parse_auth("{}");
        assert!(!status.logged_in);
        assert!(status.name.is_empty());
        assert!(status.email.is_empty());
    }

    #[test]
    fn login_url_rejects_codes_tokens_and_plain_http() {
        assert!(safe_login_url("https://accounts.example.com/oauth/authorize?client_id=abc"));
        assert!(!safe_login_url("https://accounts.example.com/callback?code=secret"));
        assert!(!safe_login_url("https://accounts.example.com/callback?token=secret"));
        assert!(!safe_login_url("http://127.0.0.1/login"));
        assert!(!safe_login_url(&format!("https://example.com/{}", "a".repeat(500))));
        let text = "请打开 https://accounts.example.com/login 完成";
        assert_eq!(
            extract_login_urls(text),
            vec!["https://accounts.example.com/login".to_string()]
        );
    }
}
