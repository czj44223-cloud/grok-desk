use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::acp::{self, AgentEvent, Incoming, ResponseBody};

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub struct Agent {
    shared: Arc<Mutex<Inner>>,
    app: AppHandle,
}

struct Inner {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    session_id: Option<String>,
    next_id: i64,
    pending: HashMap<String, Sender<ResponseBody>>,
    permissions: HashMap<String, Vec<String>>,
    prompt_id: Option<String>,
    turn_open: bool,
    starting: bool,
    shutting_down: bool,
    stderr_tail: String,
    cwd: String,
    boot: Boot,
    generation: u64,
    replay: bool,
    plugin_dirs: Vec<PathBuf>,
}

#[derive(Clone)]
enum Boot {
    New,
    Load(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeskInfo {
    pub cwd: String,
    pub session_id: String,
}

pub fn default_work_dir() -> String {
    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.is_empty() {
            return home;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return home;
        }
    }
    std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

impl Agent {
    pub fn new(app: AppHandle) -> Self {
        let plugin_dirs = app
            .path()
            .app_config_dir()
            .map(|dir| crate::plugins::saved_plugin_dirs(&dir))
            .unwrap_or_default();
        Self {
            shared: Arc::new(Mutex::new(Inner {
                child: None,
                stdin: None,
                session_id: None,
                next_id: 1,
                pending: HashMap::new(),
                permissions: HashMap::new(),
                prompt_id: None,
                turn_open: false,
                starting: false,
                shutting_down: false,
                stderr_tail: String::new(),
                cwd: default_work_dir(),
                boot: Boot::New,
                generation: 0,
                replay: false,
                plugin_dirs,
            })),
            app,
        }
    }

    pub fn shutdown(&self) {
        let mut guard = match self.shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.shutting_down = true;
        let child = guard.child.take();
        guard.stdin.take();
        drop(guard);
        if let Some(mut child) = child {
            stop_process(&mut child);
        }
    }

    pub fn send(&self, text: String) -> Result<(), String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("请输入内容。".into());
        }
        self.ensure_session()?;
        let mut guard = lock(&self.shared)?;
        if guard.turn_open {
            return Err("上一条还在进行。可以点停止，或等它结束。".into());
        }
        let session_id = guard
            .session_id
            .clone()
            .ok_or("会话还没建立。")?;
        let id = Value::from(guard.next_id);
        guard.next_id += 1;
        let key = acp::id_key(&id);
        let line = acp::request_line(&id, "session/prompt", acp::prompt_params(&session_id, &text));
        guard.prompt_id = Some(key);
        guard.turn_open = true;
        if let Err(error) = guard.write_line(&line) {
            guard.turn_open = false;
            guard.prompt_id = None;
            return Err(error);
        }
        Ok(())
    }

    pub fn cancel(&self) -> Result<(), String> {
        let mut guard = lock(&self.shared)?;
        if guard.turn_open {
            if let Some(session_id) = guard.session_id.clone() {
                let line = acp::cancel_notification(&session_id);
                guard.write_line(&line)?;
            }
            let keys: Vec<String> = guard.permissions.keys().cloned().collect();
            for key in keys {
                if let Some(id) = acp::id_from_key(&key) {
                    let line = acp::permission_cancelled(&id);
                    let _ = guard.write_line(&line);
                }
            }
            guard.permissions.clear();
            return Ok(());
        }
        drop(guard);
        self.stop_child();
        Ok(())
    }

    pub fn info(&self) -> DeskInfo {
        let guard = match self.shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        DeskInfo {
            cwd: guard.cwd.clone(),
            session_id: guard.session_id.clone().unwrap_or_default(),
        }
    }

    pub fn set_project(&self, cwd: String) -> Result<(), String> {
        let cwd = cwd.trim().to_string();
        if !std::path::Path::new(&cwd).is_dir() {
            return Err("这个文件夹不存在。".into());
        }
        {
            let guard = lock(&self.shared)?;
            if guard.turn_open {
                return Err("上一条还在进行。可以点停止，或等它结束。".into());
            }
            if crate::catalog::paths_eq(&guard.cwd, &cwd) {
                drop(guard);
                self.remember(&cwd);
                return Ok(());
            }
        }
        self.stop_child();
        {
            let mut guard = lock(&self.shared)?;
            guard.cwd = cwd.clone();
            guard.boot = Boot::New;
        }
        self.remember(&cwd);
        let _ = self.app.emit(
            "agent-event",
            AgentEvent::Reset {
                cwd,
                session_id: String::new(),
            },
        );
        Ok(())
    }

    pub fn new_session(&self) -> Result<(), String> {
        let cwd = {
            let guard = lock(&self.shared)?;
            if guard.turn_open {
                return Err("上一条还在进行。可以点停止，或等它结束。".into());
            }
            guard.cwd.clone()
        };
        self.stop_child();
        {
            let mut guard = lock(&self.shared)?;
            guard.boot = Boot::New;
        }
        let _ = self.app.emit(
            "agent-event",
            AgentEvent::Reset {
                cwd,
                session_id: String::new(),
            },
        );
        Ok(())
    }

    pub fn load_session(&self, session_id: String) -> Result<(), String> {
        let session_id = session_id.trim().to_string();
        if session_id.is_empty() {
            return Err("没有会话编号。".into());
        }
        let cwd = {
            let guard = lock(&self.shared)?;
            if guard.turn_open {
                return Err("上一条还在进行。可以点停止，或等它结束。".into());
            }
            guard.cwd.clone()
        };
        self.stop_child();
        {
            let mut guard = lock(&self.shared)?;
            guard.boot = Boot::Load(session_id.clone());
        }
        let _ = self.app.emit(
            "agent-event",
            AgentEvent::Reset {
                cwd,
                session_id: session_id.clone(),
            },
        );
        if let Err(error) = self.ensure_session() {
            let _ = self.app.emit(
                "agent-event",
                AgentEvent::Error {
                    title: "没有打开会话".into(),
                    detail: error.clone(),
                },
            );
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_session(&self, cwd: String, session_id: String) -> Result<(), String> {
        let cwd = cwd.trim().to_string();
        let session_id = session_id.trim().to_string();
        if !crate::catalog::valid_session_id(&session_id) {
            return Err("会话编号不对。".into());
        }
        let current = {
            let guard = lock(&self.shared)?;
            if guard.turn_open {
                return Err("上一条还在进行。可以点停止，或等它结束。".into());
            }
            if !crate::catalog::paths_eq(&guard.cwd, &cwd) {
                return Err("先切到这个会话所在的文件夹。".into());
            }
            guard.session_id.clone()
        };
        crate::catalog::session_directory(&crate::catalog::grok_home(), std::path::Path::new(&cwd), &session_id)?;
        let was_current = current.as_deref() == Some(session_id.as_str());
        if was_current {
            self.stop_child();
        }
        if let Err(error) = crate::catalog::delete_session_cli(std::path::Path::new(&cwd), &session_id) {
            if was_current {
                if let Ok(mut guard) = lock(&self.shared) {
                    guard.boot = Boot::Load(session_id);
                }
                return Err(format!("进程已停，但会话文件还在。\n{error}"));
            }
            return Err(error);
        }
        if let Ok(dir) = self.app.path().app_config_dir() {
            let _ = crate::catalog::set_archived(&dir.join("archived-sessions.json"), &cwd, &session_id, false);
        }
        if was_current {
            if let Ok(mut guard) = lock(&self.shared) {
                guard.boot = Boot::New;
            }
            let _ = self.app.emit(
                "agent-event",
                AgentEvent::Reset {
                    cwd,
                    session_id: String::new(),
                },
            );
        }
        Ok(())
    }

    fn remember(&self, cwd: &str) {
        let Ok(dir) = self.app.path().app_config_dir() else {
            return;
        };
        let _ = std::fs::create_dir_all(&dir);
        let _ = crate::catalog::push_recent(&dir.join("recent-projects.json"), cwd);
    }

    fn stop_child(&self) {
        let mut guard = match self.shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.generation += 1;
        let child = guard.child.take();
        guard.stdin.take();
        guard.session_id = None;
        guard.prompt_id = None;
        guard.turn_open = false;
        guard.replay = false;
        guard.permissions.clear();
        let pending = std::mem::take(&mut guard.pending);
        drop(guard);
        for (_, sender) in pending {
            let _ = sender.send(ResponseBody::Err {
                code: -1,
                message: "命令行已退出。".into(),
            });
        }
        if let Some(mut child) = child {
            stop_process(&mut child);
        }
    }

    pub fn reply_permission(&self, request_id: String, option_id: String) -> Result<(), String> {
        let mut guard = lock(&self.shared)?;
        let allowed = guard
            .permissions
            .get(&request_id)
            .cloned()
            .ok_or("这次确认已经处理过了。")?;
        if !allowed.iter().any(|id| id == &option_id) {
            return Err("没有这个选项。".into());
        }
        let id = acp::id_from_key(&request_id).ok_or("确认编号无法识别。")?;
        let line = acp::permission_selected(&id, &option_id);
        guard.permissions.remove(&request_id);
        guard.write_line(&line)
    }

    fn ensure_session(&self) -> Result<(), String> {
        {
            let mut guard = lock(&self.shared)?;
            if guard.session_id.is_some() && guard.is_alive() {
                return Ok(());
            }
            if guard.starting {
                return Err("正在启动命令行，请稍等。".into());
            }
            guard.starting = true;
        }
        let started = self.start_session();
        if let Ok(mut guard) = self.shared.lock() {
            guard.starting = false;
            if started.is_err() {
                guard.generation += 1;
                guard.session_id = None;
                guard.boot = Boot::New;
                guard.replay = false;
                let child = guard.child.take();
                guard.stdin.take();
                drop(guard);
                if let Some(mut child) = child {
                    stop_process(&mut child);
                }
            }
        }
        started
    }

    fn start_session(&self) -> Result<(), String> {
        self.spawn_process()?;
        self.round_trip(
            "initialize",
            acp::initialize_params(),
            Duration::from_secs(30),
            "命令行没有在 30 秒内完成握手。",
        )?;
        let (cwd, boot) = {
            let guard = lock(&self.shared)?;
            (guard.cwd.clone(), guard.boot.clone())
        };
        let created = match boot {
            Boot::New => self.round_trip(
                "session/new",
                acp::session_new_params(&cwd),
                Duration::from_secs(90),
                "建立会话超过 90 秒。",
            )?,
            Boot::Load(session_id) => {
                if let Ok(mut guard) = self.shared.lock() {
                    guard.replay = true;
                }
                let result = self.round_trip(
                    "session/load",
                    acp::session_load_params(&cwd, &session_id),
                    Duration::from_secs(90),
                    "打开会话超过 90 秒。",
                );
                if let Ok(mut guard) = self.shared.lock() {
                    guard.replay = false;
                }
                result?
            }
        };
        let session_id = created
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("命令行没有返回会话编号。")?
            .to_string();
        lock(&self.shared)?.session_id = Some(session_id);
        Ok(())
    }

    pub fn ensure_idle(&self) -> Result<(), String> {
        let guard = lock(&self.shared)?;
        if guard.turn_open || guard.starting {
            return Err("上一条还在进行。可以点停止，或等它结束。".into());
        }
        Ok(())
    }

    pub fn plugin_dirs(&self) -> Result<Vec<PathBuf>, String> {
        Ok(lock(&self.shared)?.plugin_dirs.clone())
    }

    pub fn apply_plugin_dirs(&self, dirs: Vec<PathBuf>) -> Result<bool, String> {
        let (session_id, cwd) = {
            let mut guard = lock(&self.shared)?;
            if guard.turn_open || guard.starting {
                return Err("上一条还在进行。可以点停止，或等它结束。".into());
            }
            guard.plugin_dirs = dirs;
            (guard.session_id.clone(), guard.cwd.clone())
        };
        let Some(session_id) = session_id else {
            let alive = lock(&self.shared)?.is_alive();
            if alive {
                self.stop_child();
            }
            return Ok(false);
        };
        self.stop_child();
        {
            let mut guard = lock(&self.shared)?;
            guard.boot = Boot::Load(session_id.clone());
        }
        let _ = self.app.emit(
            "agent-event",
            AgentEvent::Reset {
                cwd,
                session_id: session_id.clone(),
            },
        );
        if let Err(error) = self.ensure_session() {
            if let Ok(mut guard) = lock(&self.shared) {
                guard.boot = Boot::Load(session_id);
            }
            let _ = self.app.emit(
                "agent-event",
                AgentEvent::Error {
                    title: "没有打开会话".into(),
                    detail: error.clone(),
                },
            );
            return Err(error);
        }
        Ok(true)
    }

    fn spawn_process(&self) -> Result<(), String> {
        let path = crate::cli::find_grok().ok_or_else(|| "没有找到 grok.exe。".to_string())?;
        let plugin_dirs = lock(&self.shared)?.plugin_dirs.clone();
        let mut command = Command::new(&path);
        command
            .args(crate::plugins::agent_launch_args(&plugin_dirs))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|error| {
            format!("无法启动 {}：{error}", path.display())
        })?;
        let stdout = child.stdout.take().ok_or("无法读取命令行输出。")?;
        let stderr = child.stderr.take().ok_or("无法读取命令行错误输出。")?;
        let stdin = child.stdin.take().ok_or("无法向命令行写入。")?;
        let generation = {
            let mut guard = lock(&self.shared)?;
            guard.child = Some(child);
            guard.stdin = Some(stdin);
            guard.session_id = None;
            guard.prompt_id = None;
            guard.turn_open = false;
            guard.permissions.clear();
            guard.stderr_tail.clear();
            guard.replay = false;
            guard.generation += 1;
            guard.generation
        };
        let shared = Arc::clone(&self.shared);
        let app = self.app.clone();
        std::thread::Builder::new()
            .name("grok-stdout".into())
            .spawn(move || read_stdout(stdout, shared, app, generation))
            .map_err(|error| format!("无法读取命令行输出：{error}"))?;
        let shared = Arc::clone(&self.shared);
        std::thread::Builder::new()
            .name("grok-stderr".into())
            .spawn(move || read_stderr(stderr, shared))
            .map_err(|error| format!("无法读取命令行错误输出：{error}"))?;
        Ok(())
    }

    fn round_trip(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        timeout_message: &str,
    ) -> Result<Value, String> {
        let (key, receiver) = {
            let mut guard = lock(&self.shared)?;
            let id = Value::from(guard.next_id);
            guard.next_id += 1;
            let key = acp::id_key(&id);
            let line = acp::request_line(&id, method, params);
            let (sender, receiver) = mpsc::channel();
            guard.pending.insert(key.clone(), sender);
            if let Err(error) = guard.write_line(&line) {
                guard.pending.remove(&key);
                return Err(error);
            }
            (key, receiver)
        };
        match receiver.recv_timeout(timeout) {
            Ok(ResponseBody::Ok(value)) => Ok(value),
            Ok(ResponseBody::Err { message, .. }) => Err(self.with_stderr(message)),
            Err(_) => {
                if let Ok(mut guard) = self.shared.lock() {
                    guard.pending.remove(&key);
                }
                Err(self.with_stderr(timeout_message.to_string()))
            }
        }
    }

    fn with_stderr(&self, message: String) -> String {
        let Ok(guard) = self.shared.lock() else {
            return message;
        };
        let tail = guard.stderr_tail.trim();
        if tail.is_empty() {
            message
        } else {
            format!("{message}\n{tail}")
        }
    }
}

impl Inner {
    fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn write_line(&mut self, line: &str) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("命令行没有在运行。")?;
        stdin
            .write_all(line.as_bytes())
            .map_err(|error| format!("写入命令行失败：{error}"))?;
        stdin
            .flush()
            .map_err(|error| format!("写入命令行失败：{error}"))?;
        Ok(())
    }
}

fn lock(shared: &Mutex<Inner>) -> Result<std::sync::MutexGuard<'_, Inner>, String> {
    shared
        .lock()
        .map_err(|_| "内部状态锁坏了。".to_string())
}

fn read_stdout(stdout: impl Read, shared: Arc<Mutex<Inner>>, app: AppHandle, generation: u64) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut buffer = Vec::new();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                if buffer.last() == Some(&b'\n') {
                    buffer.pop();
                }
                if buffer.last() == Some(&b'\r') {
                    buffer.pop();
                }
                let line = String::from_utf8_lossy(&buffer);
                if !line.trim().is_empty() {
                    dispatch(&line, &shared, &app);
                }
            }
            Err(_) => break,
        }
    }
    finish_reader(&shared, &app, generation);
}

fn read_stderr(stderr: impl Read, shared: Arc<Mutex<Inner>>) {
    let mut reader = BufReader::new(stderr);
    loop {
        let mut buffer = Vec::new();
        match reader.read_until(b'\n', &mut buffer) {
            Ok(0) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buffer);
                if let Ok(mut guard) = shared.lock() {
                    guard.stderr_tail.push_str(&line);
                    if guard.stderr_tail.chars().count() > 4000 {
                        let tail: String = guard.stderr_tail.chars().rev().take(4000).collect();
                        guard.stderr_tail = tail.chars().rev().collect();
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn dispatch(line: &str, shared: &Mutex<Inner>, app: &AppHandle) {
    match acp::parse_line(line) {
        Incoming::Response { id, body } => handle_response(id, body, shared, app),
        Incoming::Request { id, method, params } => handle_request(id, &method, &params, shared, app),
        Incoming::Notification { method, params } => {
            if method == "session/update" {
                let replay = shared.lock().map(|guard| guard.replay).unwrap_or(false);
                if let Some(event) = acp::event_from_update(&params).and_then(|event| acp::visible_event(event, replay)) {
                    let _ = app.emit("agent-event", event);
                }
            } else if let Some(event) = acp::note_from_notification(&method, &params) {
                let _ = app.emit("agent-event", event);
            }
        }
        Incoming::Invalid => {}
    }
}

fn handle_response(id: Value, body: ResponseBody, shared: &Mutex<Inner>, app: &AppHandle) {
    let key = acp::id_key(&id);
    let mut guard = match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.prompt_id.as_deref() == Some(key.as_str()) {
        guard.prompt_id = None;
        guard.turn_open = false;
        guard.permissions.clear();
        drop(guard);
        match body {
            ResponseBody::Ok(value) => {
                let _ = app.emit(
                    "agent-event",
                    AgentEvent::Turn {
                        stop_reason: acp::stop_reason(&value),
                    },
                );
            }
            ResponseBody::Err { message, .. } => {
                let _ = app.emit(
                    "agent-event",
                    AgentEvent::Error {
                        title: "这一轮没有完成".into(),
                        detail: message,
                    },
                );
            }
        }
        return;
    }
    if let Some(sender) = guard.pending.remove(&key) {
        drop(guard);
        let _ = sender.send(body);
    }
}

fn handle_request(id: Value, method: &str, params: &Value, shared: &Mutex<Inner>, app: &AppHandle) {
    if method == "session/request_permission" {
        let event = acp::permission_event(&id, params);
        let options = acp::permission_option_ids(&event);
        if let Ok(mut guard) = shared.lock() {
            if let AgentEvent::Permission { request_id, .. } = &event {
                guard.permissions.insert(request_id.clone(), options);
            }
        }
        let _ = app.emit("agent-event", event);
        return;
    }
    let line = acp::error_response(&id, -32601, "桌面端还没有实现这个请求。");
    if let Ok(mut guard) = shared.lock() {
        let _ = guard.write_line(&line);
    }
}

fn finish_reader(shared: &Mutex<Inner>, app: &AppHandle, generation: u64) {
    let mut guard = match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.generation != generation {
        return;
    }
    let quiet = guard.shutting_down;
    let turn = guard.turn_open;
    let tail = guard.stderr_tail.trim().to_string();
    guard.turn_open = false;
    guard.prompt_id = None;
    guard.session_id = None;
    let child = guard.child.take();
    guard.stdin.take();
    guard.permissions.clear();
    let pending = std::mem::take(&mut guard.pending);
    drop(guard);
    if let Some(mut child) = child {
        if matches!(child.try_wait(), Ok(None)) {
            stop_process(&mut child);
        } else {
            let _ = child.wait();
        }
    }
    let message = if tail.is_empty() {
        "命令行已退出。".to_string()
    } else {
        format!("命令行已退出。\n{tail}")
    };
    for (_, sender) in pending {
        let _ = sender.send(ResponseBody::Err {
            code: -1,
            message: message.clone(),
        });
    }
    if turn && !quiet {
        let _ = app.emit(
            "agent-event",
            AgentEvent::Error {
                title: "命令行已退出".into(),
                detail: message,
            },
        );
    }
}

fn stop_process(child: &mut Child) {
    let pid = child.id();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[tauri::command]
pub fn desk_info(agent: tauri::State<'_, Agent>) -> DeskInfo {
    agent.info()
}

#[tauri::command]
pub fn agent_set_project(agent: tauri::State<'_, Agent>, cwd: String) -> Result<(), String> {
    agent.set_project(cwd)
}

#[tauri::command]
pub fn agent_new_session(agent: tauri::State<'_, Agent>) -> Result<(), String> {
    agent.new_session()
}

#[tauri::command]
pub fn agent_load_session(agent: tauri::State<'_, Agent>, session_id: String) -> Result<(), String> {
    agent.load_session(session_id)
}

#[tauri::command]
pub fn delete_session(agent: tauri::State<'_, Agent>, cwd: String, session_id: String) -> Result<(), String> {
    agent.delete_session(cwd, session_id)
}

#[tauri::command]
pub fn agent_send(agent: tauri::State<'_, Agent>, text: String) -> Result<(), String> {
    agent.send(text)
}

#[tauri::command]
pub fn agent_cancel(agent: tauri::State<'_, Agent>) -> Result<(), String> {
    agent.cancel()
}

#[tauri::command]
pub fn agent_reply_permission(
    agent: tauri::State<'_, Agent>,
    request_id: String,
    option_id: String,
) -> Result<(), String> {
    agent.reply_permission(request_id, option_id)
}

pub fn shutdown_from(app: &AppHandle) {
    if let Some(agent) = app.try_state::<Agent>() {
        agent.shutdown();
    }
}
