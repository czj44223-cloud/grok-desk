use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MIN_VERSION: (u64, u64, u64) = (1, 0, 70);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliReport {
    pub ok: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub title: String,
    pub detail: String,
    pub steps: Vec<String>,
}

#[derive(Debug)]
enum RunError {
    Spawn(String),
    Timeout,
}

pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_digit() {
            let start = index;
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
                index += 1;
            }
            if let Some(version) = parse_version_token(&text[start..index]) {
                return Some(version);
            }
        } else {
            index += 1;
        }
    }
    None
}

fn parse_version_token(token: &str) -> Option<(u64, u64, u64)> {
    let mut parts = token.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

pub fn version_at_least(found: (u64, u64, u64), minimum: (u64, u64, u64)) -> bool {
    found >= minimum
}

pub fn format_version(version: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

pub fn find_grok() -> Option<PathBuf> {
    if let Some(path) = find_on_path("grok.exe") {
        return Some(path);
    }
    for candidate in fallback_candidates() {
        if is_runnable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn fallback_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        paths.push(PathBuf::from(home).join(".grok").join("bin").join("grok.exe"));
    }
    if let Ok(home) = std::env::var("GROK_HOME") {
        paths.push(PathBuf::from(home).join("bin").join("grok.exe"));
    }
    paths
}

fn find_on_path(file_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(file_name);
        if is_runnable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_runnable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
}

pub fn check_cli() -> CliReport {
    let Some(path) = find_grok() else {
        return report_missing();
    };
    let path_text = path.display().to_string();
    match capture_version(&path) {
        Err(RunError::Spawn(message)) => report_spawn_error(&path_text, &message),
        Err(RunError::Timeout) => report_timeout(&path_text),
        Ok((code, text)) => report_from_output(&path_text, code, &text),
    }
}

pub fn report_from_output(path: &str, exit_code: i32, text: &str) -> CliReport {
    let Some(version) = parse_version(text) else {
        return report_unparsed(path, exit_code, text);
    };
    let version_text = format_version(version);
    if version_at_least(version, MIN_VERSION) {
        report_ok(path, &version_text)
    } else {
        report_too_old(path, &version_text)
    }
}

fn report_missing() -> CliReport {
    CliReport {
        ok: false,
        path: None,
        version: None,
        title: "没有找到 Grok 命令行".to_string(),
        detail: "桌面端不会自己回答问题，它要调用本机已经安装的 Grok 命令行。现在找不到 grok.exe。".to_string(),
        steps: vec![
            "打开 PowerShell，运行：irm https://x.ai/cli/install.ps1 | iex".to_string(),
            "安装结束后，关掉这个窗口再打开。".to_string(),
            "如果装过了仍提示找不到，确认用户目录下的 .grok\\bin 在 PATH 里，然后重开本应用。".to_string(),
        ],
    }
}

fn report_too_old(path: &str, version: &str) -> CliReport {
    let minimum = format_version(MIN_VERSION);
    CliReport {
        ok: false,
        path: Some(path.to_string()),
        version: Some(version.to_string()),
        title: "Grok 命令行版本过低".to_string(),
        detail: format!(
            "当前版本是 {version}，桌面端需要 {minimum} 或更高。这个版本才有后续要用的 agent stdio 接口。"
        ),
        steps: vec![
            "打开 PowerShell，运行：grok update".to_string(),
            "如果这个 Grok 是用 WinGet 安装的，先关掉本应用，再运行：winget upgrade --id xAI.GrokBuild -e".to_string(),
            "升级后重开本应用。可用 grok --version 先自己看一眼版本。".to_string(),
        ],
    }
}

fn report_unparsed(path: &str, exit_code: i32, text: &str) -> CliReport {
    let preview = preview_output(text);
    CliReport {
        ok: false,
        path: Some(path.to_string()),
        version: None,
        title: "无法确认 Grok 版本".to_string(),
        detail: format!("已经找到命令，但 grok --version 的输出认不出来（退出码 {exit_code}）。输出是：{preview}"),
        steps: vec![
            "在 PowerShell 里运行 grok --version，看它是否真的打印了版本号。".to_string(),
            "如果命令报错或一直不动，运行 grok update 后再试。".to_string(),
            "确认上面显示的路径就是你平时使用的那个 grok.exe。".to_string(),
        ],
    }
}

fn report_ok(path: &str, version: &str) -> CliReport {
    CliReport {
        ok: true,
        path: Some(path.to_string()),
        version: Some(version.to_string()),
        title: "命令行可以用".to_string(),
        detail: format!("已找到 Grok {version}。这一步只确认环境。对话、会话和权限会在后面的阶段接上。"),
        steps: Vec::new(),
    }
}

fn report_spawn_error(path: &str, message: &str) -> CliReport {
    CliReport {
        ok: false,
        path: Some(path.to_string()),
        version: None,
        title: "无法运行 Grok 命令行".to_string(),
        detail: format!("找到了文件，但启动失败：{message}"),
        steps: vec![
            "确认这个文件没有被安全软件拦截。".to_string(),
            "在 PowerShell 里直接运行 grok --version。能跑通的话，重开本应用再检查一次。".to_string(),
            "仍然失败就重新安装：irm https://x.ai/cli/install.ps1 | iex".to_string(),
        ],
    }
}

fn report_timeout(path: &str) -> CliReport {
    CliReport {
        ok: false,
        path: Some(path.to_string()),
        version: None,
        title: "检查 Grok 版本超时".to_string(),
        detail: "grok --version 超过 20 秒没有返回。桌面端停掉了这次检查，避免窗口一直转圈。".to_string(),
        steps: vec![
            "在 PowerShell 里运行 grok --version，看它是不是在等登录或卡在更新。".to_string(),
            "处理完之后，回到这里点「重新检查」。".to_string(),
        ],
    }
}

fn preview_output(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = one_line.chars();
    let clipped: String = chars.by_ref().take(180).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else if clipped.is_empty() {
        "（没有输出）".to_string()
    } else {
        clipped
    }
}

fn capture_version(path: &Path) -> Result<(i32, String), RunError> {
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // CREATE_NO_WINDOW: a version check must not flash a console over the window.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|error| RunError::Spawn(error.to_string()))?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = std::io::Read::read_to_end(pipe, &mut out);
        }
        if let Some(pipe) = stderr.as_mut() {
            let _ = std::io::Read::read_to_end(pipe, &mut err);
        }
        (out, err)
    });

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (out, err) = reader.join().unwrap_or_else(|_| (Vec::new(), Vec::new()));
                let text = format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&out),
                    String::from_utf8_lossy(&err)
                );
                return Ok((status.code().unwrap_or(-1), text));
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(RunError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(error) => return Err(RunError::Spawn(error.to_string())),
        }
    }
}

pub(crate) struct Captured {
    pub code: i32,
    pub stdout: String,
    #[allow(dead_code)]
    pub stderr: String,
}

pub(crate) fn kill_tree(child: &mut std::process::Child) {
    let pid = child.id();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs grok and returns its output. Callers must not show that output when it can hold a login secret.
pub(crate) fn run_grok(args: &[&str], timeout: Duration) -> Result<Captured, String> {
    let path = find_grok().ok_or("没有找到 grok.exe。".to_string())?;
    let mut command = Command::new(path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|_| "无法启动命令行。".to_string())?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = std::io::Read::read_to_end(pipe, &mut out);
        }
        if let Some(pipe) = stderr.as_mut() {
            let _ = std::io::Read::read_to_end(pipe, &mut err);
        }
        (out, err)
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (out, err) = reader.join().unwrap_or_else(|_| (Vec::new(), Vec::new()));
                return Ok(Captured {
                    code: status.code().unwrap_or(-1),
                    stdout: String::from_utf8_lossy(&out).into_owned(),
                    stderr: String::from_utf8_lossy(&err).into_owned(),
                });
            }
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(&mut child);
                let _ = reader.join();
                return Err("命令行超时。".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => return Err("无法启动命令行。".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_line_this_machine_prints() {
        assert_eq!(parse_version("grok 1.0.70 (f50904eaee6c)\n"), Some((1, 0, 70)));
    }

    #[test]
    fn reads_a_version_surrounded_by_other_text() {
        assert_eq!(parse_version("prefix grok 2.3.4 suffix"), Some((2, 3, 4)));
    }

    #[test]
    fn ignores_text_without_a_version() {
        assert_eq!(parse_version("not installed"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn compares_against_the_minimum() {
        assert!(version_at_least((1, 0, 70), MIN_VERSION));
        assert!(version_at_least((1, 1, 0), MIN_VERSION));
        assert!(version_at_least((2, 0, 0), MIN_VERSION));
        assert!(!version_at_least((1, 0, 69), MIN_VERSION));
        assert!(!version_at_least((0, 9, 99), MIN_VERSION));
    }

    #[test]
    fn ok_report_names_the_version_and_path() {
        let report = report_from_output(r"C:\Users\me\.grok\bin\grok.exe", 0, "grok 1.0.70 (abc)");
        assert!(report.ok);
        assert_eq!(report.version.as_deref(), Some("1.0.70"));
        assert_eq!(report.path.as_deref(), Some(r"C:\Users\me\.grok\bin\grok.exe"));
        assert!(report.steps.is_empty());
        assert!(report.title.contains("可以用"));
    }

    #[test]
    fn old_version_explains_how_to_upgrade() {
        let report = report_from_output(r"D:\grok.exe", 0, "grok 1.0.69");
        assert!(!report.ok);
        assert_eq!(report.version.as_deref(), Some("1.0.69"));
        assert!(report.detail.contains("1.0.70"));
        assert!(report.steps.iter().any(|step| step.contains("grok update")));
    }

    #[test]
    fn unreadable_output_is_not_treated_as_success() {
        let report = report_from_output(r"D:\grok.exe", 1, "error: boom");
        assert!(!report.ok);
        assert!(report.version.is_none());
        assert!(report.detail.contains("boom"));
        assert!(report.detail.contains("退出码 1"));
    }

    #[test]
    fn missing_binary_tells_the_user_the_install_command() {
        let report = report_missing();
        assert!(!report.ok);
        assert!(report.path.is_none());
        let joined = report.steps.join("\n");
        assert!(joined.contains("irm https://x.ai/cli/install.ps1 | iex"));
    }
}
