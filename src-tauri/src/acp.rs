use serde::Serialize;
use serde_json::{json, Value};

/// One line from `grok agent stdio`.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    Response { id: Value, body: ResponseBody },
    Request { id: Value, method: String, params: Value },
    Notification { method: String, params: Value },
    Invalid,
}

#[derive(Debug, PartialEq)]
pub enum ResponseBody {
    Ok(Value),
    Err { code: i64, message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    Message { text: String },
    Thought { text: String },
    Tool {
        id: String,
        title: String,
        kind: String,
        status: String,
        detail: String,
        partial: bool,
        subagent: bool,
    },
    Plan { entries: Vec<PlanItem> },
    Permission {
        #[serde(rename = "requestId")]
        request_id: String,
        title: String,
        kind: String,
        options: Vec<PermOption>,
    },
    Turn {
        #[serde(rename = "stopReason")]
        stop_reason: String,
    },
    Error { title: String, detail: String },
    User { text: String },
    Note { text: String },
    Reset {
        cwd: String,
        #[serde(rename = "sessionId")]
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanItem {
    pub content: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PermOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}

pub fn parse_line(line: &str) -> Incoming {
    let value: Value = match serde_json::from_str(line.trim()) {
        Ok(value) => value,
        Err(_) => return Incoming::Invalid,
    };
    if !value.is_object() {
        return Incoming::Invalid;
    }
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        if let Some(id) = value.get("id").filter(|id| !id.is_null()) {
            return Incoming::Request {
                id: id.clone(),
                method: method.to_string(),
                params,
            };
        }
        return Incoming::Notification {
            method: method.to_string(),
            params,
        };
    }
    let Some(id) = value.get("id").filter(|id| !id.is_null()) else {
        return Incoming::Invalid;
    };
    if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-1);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("命令行返回了错误")
            .to_string();
        return Incoming::Response {
            id: id.clone(),
            body: ResponseBody::Err { code, message },
        };
    }
    Incoming::Response {
        id: id.clone(),
        body: ResponseBody::Ok(value.get("result").cloned().unwrap_or(Value::Null)),
    }
}

pub fn event_from_update(params: &Value) -> Option<AgentEvent> {
    let update = params.get("update")?;
    match update.get("sessionUpdate").and_then(Value::as_str)? {
        "agent_message_chunk" => chunk_text(update).map(|text| AgentEvent::Message { text }),
        "agent_thought_chunk" => chunk_text(update).map(|text| AgentEvent::Thought { text }),
        "user_message_chunk" => chunk_text(update).map(|text| AgentEvent::User { text }),
        "tool_call" => Some(tool_event(update, false)),
        "tool_call_update" => Some(tool_event(update, true)),
        "plan" => plan_event(update),
        _ => None,
    }
}

pub fn visible_event(event: AgentEvent, replay: bool) -> Option<AgentEvent> {
    match event {
        AgentEvent::User { .. } if !replay => None,
        other => Some(other),
    }
}

pub fn note_from_notification(method: &str, params: &Value) -> Option<AgentEvent> {
    if method == "x.ai/git/worktree/status" {
        let text = serde_json::to_string(params).unwrap_or_default();
        return Some(AgentEvent::Note {
            text: format!("工作树：{}", truncate(&text, 240)),
        });
    }
    if method.to_ascii_lowercase().contains("hook") {
        let text = serde_json::to_string(params).unwrap_or_default();
        return Some(AgentEvent::Note {
            text: format!("钩子：{}", truncate(&text, 240)),
        });
    }
    None
}

pub fn permission_event(id: &Value, params: &Value) -> AgentEvent {
    let tool = params.get("toolCall");
    let title = tool
        .and_then(|item| item.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("需要你确认一项操作")
        .to_string();
    let kind = tool
        .and_then(|item| item.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let id = item.get("optionId").and_then(Value::as_str)?.to_string();
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let kind = item
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    Some(PermOption { id, name, kind })
                })
                .collect()
        })
        .unwrap_or_default();
    AgentEvent::Permission {
        request_id: id_key(id),
        title,
        kind,
        options,
    }
}

pub fn permission_option_ids(event: &AgentEvent) -> Vec<String> {
    match event {
        AgentEvent::Permission { options, .. } => options.iter().map(|option| option.id.clone()).collect(),
        _ => Vec::new(),
    }
}

pub fn id_key(id: &Value) -> String {
    if let Some(number) = id.as_i64() {
        return format!("n:{number}");
    }
    if let Some(text) = id.as_str() {
        return format!("s:{text}");
    }
    format!("j:{id}")
}

pub fn id_from_key(key: &str) -> Option<Value> {
    if let Some(rest) = key.strip_prefix("n:") {
        let number: i64 = rest.parse().ok()?;
        return Some(Value::from(number));
    }
    key.strip_prefix("s:").map(|rest| Value::String(rest.to_string()))
}

pub fn stop_reason(result: &Value) -> String {
    result
        .get("stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
        .to_string()
}

pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": 1,
        "clientCapabilities": {},
        "clientInfo": {
            "name": "grok-desk",
            "title": "Grok 桌面",
            "version": "0.1.0"
        }
    })
}

pub fn session_new_params(cwd: &str) -> Value {
    json!({
        "cwd": cwd,
        "mcpServers": []
    })
}

pub fn session_load_params(cwd: &str, session_id: &str) -> Value {
    json!({
        "sessionId": session_id,
        "cwd": cwd,
        "mcpServers": []
    })
}

pub fn prompt_params(session_id: &str, text: &str) -> Value {
    json!({
        "sessionId": session_id,
        "prompt": [{ "type": "text", "text": text }]
    })
}

pub fn request_line(id: &Value, method: &str, params: Value) -> String {
    encode(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    }))
}

pub fn cancel_notification(session_id: &str) -> String {
    encode(&json!({
        "jsonrpc": "2.0",
        "method": "session/cancel",
        "params": { "sessionId": session_id }
    }))
}

pub fn permission_selected(id: &Value, option_id: &str) -> String {
    encode(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "outcome": {
                "outcome": "selected",
                "optionId": option_id
            }
        }
    }))
}

pub fn permission_cancelled(id: &Value) -> String {
    encode(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "outcome": { "outcome": "cancelled" }
        }
    }))
}

pub fn error_response(id: &Value, code: i64, message: &str) -> String {
    encode(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    }))
}

fn encode(value: &Value) -> String {
    let mut line = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    line.push('\n');
    line
}

fn chunk_text(update: &Value) -> Option<String> {
    let content = update.get("content")?;
    let text = if let Some(text) = content.as_str() {
        text.to_string()
    } else {
        content.get("text").and_then(Value::as_str)?.to_string()
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn tool_event(update: &Value, partial: bool) -> AgentEvent {
    let id = update
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let title = update.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let title = if title.is_empty() && !partial {
        "工具".to_string()
    } else {
        title
    };
    let kind = update.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    let status = update.get("status").and_then(Value::as_str).unwrap_or("").to_string();
    let subagent = is_subagent(update);
    let detail = if subagent {
        subagent_detail(update).or_else(|| tool_detail(update)).unwrap_or_default()
    } else {
        tool_detail(update).unwrap_or_default()
    };
    AgentEvent::Tool {
        id,
        title,
        kind,
        status,
        detail,
        partial,
        subagent,
    }
}

fn is_subagent(update: &Value) -> bool {
    let title = update.get("title").and_then(Value::as_str).unwrap_or("");
    if title.eq_ignore_ascii_case("spawn_subagent") {
        return true;
    }
    for key in ["name", "toolName", "tool_name"] {
        if let Some(name) = update.get(key).and_then(Value::as_str) {
            if name == "spawn_subagent" || name.ends_with("/spawn_subagent") {
                return true;
            }
        }
    }
    let Some(raw) = update.get("rawInput") else {
        return false;
    };
    let marked = raw.get("isolation").is_some()
        || raw.get("subagent_type").is_some()
        || raw.get("subagentType").is_some()
        || raw.get("run_in_background").is_some()
        || raw.get("resume_from").is_some();
    marked && raw.get("prompt").is_some() && raw.get("description").is_some()
}

fn subagent_detail(update: &Value) -> Option<String> {
    let raw = update.get("rawInput")?;
    let description = raw.get("description").and_then(Value::as_str).unwrap_or("").trim();
    let kind = raw
        .get("subagent_type")
        .or_else(|| raw.get("subagentType"))
        .and_then(Value::as_str)
        .unwrap_or("通用");
    let isolation = match raw.get("isolation").and_then(Value::as_str) {
        Some("worktree") => "独立工作树",
        Some(_) => "当前目录",
        None => "当前目录",
    };
    let prompt = raw.get("prompt").and_then(Value::as_str).unwrap_or("");
    let mut lines = Vec::new();
    if !description.is_empty() {
        lines.push(description.to_string());
    }
    lines.push(format!("类型 {kind} · {isolation}"));
    if !prompt.is_empty() {
        lines.push(truncate(prompt, 160));
    }
    Some(lines.join("\n"))
}

fn tool_detail(update: &Value) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(items) = update.get("content").and_then(Value::as_array) {
        for item in items {
            match item.get("type").and_then(Value::as_str) {
                Some("diff") => {
                    let path = item.get("path").and_then(Value::as_str).unwrap_or("文件");
                    parts.push(format!("改动 {path}"));
                }
                Some("terminal") => parts.push("终端输出".to_string()),
                _ => {
                    if let Some(text) = item
                        .get("content")
                        .and_then(|content| content.get("text"))
                        .and_then(Value::as_str)
                    {
                        if !text.is_empty() {
                            parts.push(text.to_string());
                        }
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        if let Some(raw) = update.get("rawInput").filter(|value| !value.is_null()) {
            let preview = serde_json::to_string(raw).unwrap_or_default();
            if !preview.is_empty() && preview != "null" {
                parts.push(preview);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(truncate(&parts.join("\n"), 2000))
    }
}

fn plan_event(update: &Value) -> Option<AgentEvent> {
    let items = update.get("entries")?.as_array()?;
    let entries = items
        .iter()
        .filter_map(|entry| {
            let content = entry.get("content").and_then(Value::as_str)?.trim();
            if content.is_empty() {
                return None;
            }
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending")
                .to_string();
            Some(PlanItem {
                content: content.to_string(),
                status,
            })
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        None
    } else {
        Some(AgentEvent::Plan { entries })
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_message_chunk() {
        let line = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"你好"}}}}"#;
        let Incoming::Notification { method, params } = parse_line(line) else {
            panic!("expected a notification");
        };
        assert_eq!(method, "session/update");
        assert_eq!(
            event_from_update(&params),
            Some(AgentEvent::Message { text: "你好".into() })
        );
    }

    #[test]
    fn reads_a_thought_chunk() {
        let params = json!({"update": {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "先看目录"}}});
        assert_eq!(
            event_from_update(&params),
            Some(AgentEvent::Thought { text: "先看目录".into() })
        );
    }

    #[test]
    fn keeps_user_text_for_replay_and_hides_it_on_a_live_turn() {
        let user = json!({"update": {"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "hi"}}});
        let event = event_from_update(&user).unwrap();
        assert_eq!(event, AgentEvent::User { text: "hi".into() });
        assert_eq!(visible_event(event.clone(), false), None);
        assert_eq!(visible_event(event, true).unwrap(), AgentEvent::User { text: "hi".into() });
        let commands = json!({"update": {"sessionUpdate": "available_commands_update"}});
        assert_eq!(event_from_update(&commands), None);
    }

    #[test]
    fn reads_a_tool_call_and_a_later_update() {
        let created = json!({"update": {
            "sessionUpdate": "tool_call",
            "toolCallId": "call_1",
            "title": "列出目录",
            "kind": "execute",
            "status": "pending",
            "rawInput": {"command": "Get-ChildItem"}
        }});
        let AgentEvent::Tool { id, title, kind, status, detail, partial, subagent } = event_from_update(&created).unwrap() else {
            panic!("tool");
        };
        assert_eq!(id, "call_1");
        assert_eq!(title, "列出目录");
        assert_eq!(kind, "execute");
        assert_eq!(status, "pending");
        assert!(!partial);
        assert!(!subagent);
        assert!(detail.contains("Get-ChildItem"));

        let update = json!({"update": {
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_1",
            "status": "completed",
            "content": [{"type": "content", "content": {"type": "text", "text": "src"}}]
        }});
        let AgentEvent::Tool { status, detail, partial, title, .. } = event_from_update(&update).unwrap() else {
            panic!("update");
        };
        assert!(partial);
        assert_eq!(status, "completed");
        assert_eq!(detail, "src");
        assert_eq!(title, "");
    }

    #[test]
    fn describes_a_diff_without_printing_the_whole_file() {
        let update = json!({"update": {
            "sessionUpdate": "tool_call_update",
            "toolCallId": "call_2",
            "content": [{"type": "diff", "path": "C:/repo/a.txt", "oldText": "a", "newText": "b"}]
        }});
        let AgentEvent::Tool { detail, .. } = event_from_update(&update).unwrap() else {
            panic!("diff");
        };
        assert_eq!(detail, "改动 C:/repo/a.txt");
        assert!(!detail.contains("\"newText\""));
    }

    #[test]
    fn reads_a_plan() {
        let params = json!({"update": {"sessionUpdate": "plan", "entries": [
            {"content": "看文件", "status": "in_progress", "priority": "high"},
            {"content": "  ", "status": "pending"}
        ]}});
        assert_eq!(
            event_from_update(&params),
            Some(AgentEvent::Plan {
                entries: vec![PlanItem { content: "看文件".into(), status: "in_progress".into() }]
            })
        );
    }

    #[test]
    fn reads_a_permission_request() {
        let line = r#"{"jsonrpc":"2.0","id":5,"method":"session/request_permission","params":{"sessionId":"s","toolCall":{"toolCallId":"call_1","title":"运行 dir","kind":"execute"},"options":[{"optionId":"allow","name":"Allow once","kind":"allow_once"},{"optionId":"reject","name":"Reject","kind":"reject_once"}]}}"#;
        let Incoming::Request { id, method, params } = parse_line(line) else {
            panic!("request");
        };
        assert_eq!(method, "session/request_permission");
        let event = permission_event(&id, &params);
        assert_eq!(
            event,
            AgentEvent::Permission {
                request_id: "n:5".into(),
                title: "运行 dir".into(),
                kind: "execute".into(),
                options: vec![
                    PermOption { id: "allow".into(), name: "Allow once".into(), kind: "allow_once".into() },
                    PermOption { id: "reject".into(), name: "Reject".into(), kind: "reject_once".into() },
                ],
            }
        );
        assert_eq!(permission_option_ids(&event), vec!["allow", "reject"]);
    }

    #[test]
    fn cancel_answers_a_pending_permission() {
        let line = permission_cancelled(&json!(5));
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert!(value.get("method").is_none());
        assert_eq!(value["id"], 5);
        assert_eq!(value["result"]["outcome"]["outcome"], "cancelled");
        assert!(value["result"]["outcome"].get("optionId").is_none());
    }

    #[test]
    fn selected_permission_keeps_the_option_id() {
        let line = permission_selected(&Value::String("abc".into()), "allow");
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["id"], "abc");
        assert_eq!(value["result"]["outcome"]["optionId"], "allow");
        assert_eq!(id_from_key("s:abc"), Some(Value::String("abc".into())));
        assert_eq!(id_from_key("n:5"), Some(json!(5)));
    }

    #[test]
    fn cancel_notification_has_no_id() {
        let line = cancel_notification("sess");
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["method"], "session/cancel");
        assert!(value.get("id").is_none());
        assert_eq!(value["params"]["sessionId"], "sess");
    }

    #[test]
    fn prompt_and_session_shapes_do_not_auto_approve() {
        let init = initialize_params();
        assert_eq!(init["protocolVersion"], 1);
        assert!(init["clientCapabilities"].as_object().unwrap().is_empty());
        let created = session_new_params("C:/Users/me");
        assert_eq!(created["cwd"], "C:/Users/me");
        assert!(created["mcpServers"].as_array().unwrap().is_empty());
        assert!(created.get("_meta").is_none());
        let loaded = session_load_params("C:/Users/me", "sess");
        assert_eq!(loaded["sessionId"], "sess");
        assert_eq!(loaded["cwd"], "C:/Users/me");
        assert!(loaded.get("_meta").is_none());
        let prompt = prompt_params("sess", "你好");
        assert_eq!(prompt["prompt"][0]["text"], "你好");
    }

    #[test]
    fn summarizes_a_subagent_spawn_without_the_whole_prompt() {
        let prompt = "x".repeat(400);
        let params = json!({"update": {
            "sessionUpdate": "tool_call",
            "toolCallId": "sub_1",
            "title": "spawn_subagent",
            "kind": "other",
            "status": "in_progress",
            "rawInput": {
                "prompt": prompt,
                "description": "看结构",
                "isolation": "worktree",
                "subagent_type": "explore"
            }
        }});
        let AgentEvent::Tool { subagent, detail, .. } = event_from_update(&params).unwrap() else {
            panic!("subagent");
        };
        assert!(subagent);
        assert!(detail.contains("看结构"));
        assert!(detail.contains("explore"));
        assert!(detail.contains("独立工作树"));
        assert!(detail.chars().count() < 250);
        let status = note_from_notification("x.ai/git/worktree/status", &json!({"phase": "copy"}));
        assert!(matches!(status, Some(AgentEvent::Note { .. })));
    }

    #[test]
    fn distinguishes_results_errors_and_noise() {
        let ok = parse_line(r#"{"jsonrpc":"2.0","id":2,"result":{"stopReason":"end_turn"}}"#);
        let Incoming::Response { body: ResponseBody::Ok(value), .. } = ok else {
            panic!("ok");
        };
        assert_eq!(stop_reason(&value), "end_turn");
        let err = parse_line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"boom"}}"#);
        assert_eq!(
            err,
            Incoming::Response {
                id: json!(2),
                body: ResponseBody::Err { code: -32000, message: "boom".into() }
            }
        );
        assert_eq!(parse_line("not json"), Incoming::Invalid);
        assert_eq!(stop_reason(&json!({})), "end_turn");
        assert_eq!(stop_reason(&json!({"stopReason": "cancelled"})), "cancelled");
    }

    #[test]
    fn unknown_client_request_gets_an_error_line() {
        let line = error_response(&json!(9), -32601, "桌面端还没有实现这个请求。");
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["id"], 9);
        assert_eq!(value["error"]["code"], -32601);
        assert!(value.get("result").is_none());
    }
}
