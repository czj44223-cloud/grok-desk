import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Side, { type LiveSubagent } from "./side";

type Phase = "idle" | "starting" | "running";

type PlanEntry = { content: string; status: string };

type PermOption = { id: string; name: string; kind: string };

type AgentEvent =
  | { type: "message"; text: string }
  | { type: "thought"; text: string }
  | {
      type: "tool";
      id: string;
      title: string;
      kind: string;
      status: string;
      detail: string;
      partial: boolean;
      subagent: boolean;
    }
  | { type: "user"; text: string }
  | { type: "note"; text: string }
  | { type: "reset"; cwd: string; sessionId: string }
  | { type: "plan"; entries: PlanEntry[] }
  | {
      type: "permission";
      requestId: string;
      title: string;
      kind: string;
      options: PermOption[];
    }
  | { type: "turn"; stopReason: string }
  | { type: "error"; title: string; detail: string };

type ToolItem = {
  kind: "tool";
  id: string;
  title: string;
  toolKind: string;
  status: string;
  detail: string;
  subagent: boolean;
};

type Item =
  | { kind: "user"; text: string }
  | { kind: "agent"; text: string }
  | { kind: "thought"; text: string }
  | ToolItem
  | { kind: "plan"; entries: PlanEntry[] }
  | { kind: "note"; tone: "bad" | "plain"; text: string };

const KIND_LABEL: Record<string, string> = {
  read: "读取",
  edit: "修改",
  delete: "删除",
  move: "移动",
  search: "搜索",
  execute: "命令",
  think: "思考",
  fetch: "联网",
  switch_mode: "切换模式",
  other: "工具",
};

const STATUS_LABEL: Record<string, string> = {
  pending: "等待",
  in_progress: "进行中",
  completed: "完成",
  failed: "失败",
  cancelled: "已停止",
};

function extractHttpUrls(text: string): string[] {
  const found: string[] = [];
  const pattern = /https?:\/\/[^\s<>"'`\\]+/gi;
  for (const match of text.matchAll(pattern)) {
    const url = match[0].replace(/[)\]}.,;!?]+$/g, "");
    if (url.length >= 8 && url.length <= 2000 && !/[\u0000-\u001f\u007f]/.test(url)) {
      found.push(url);
    }
  }
  return found;
}

function kindLabel(kind: string): string {
  if (!kind) return "工具";
  return KIND_LABEL[kind] ?? "工具";
}

function statusLabel(status: string): string {
  if (!status) return "";
  return STATUS_LABEL[status] ?? status;
}

function optionLabel(option: PermOption): string {
  switch (option.kind) {
    case "allow_once":
      return "这次允许";
    case "allow_always":
      return "这一次会话都允许";
    case "reject_once":
      return "这次拒绝";
    case "reject_always":
      return "这一次会话都拒绝";
    default:
      return option.name || "选择";
  }
}

function stopNote(reason: string): string | null {
  switch (reason) {
    case "end_turn":
      return null;
    case "cancelled":
      return "已停止。";
    case "max_tokens":
      return "这次回复达到长度上限。";
    case "max_turn_requests":
      return "这次来回次数达到上限。";
    case "refusal":
      return "命令行拒绝继续。";
    default:
      return `这次结束了（${reason}）。`;
  }
}

function mergeTool(items: Item[], event: Extract<AgentEvent, { type: "tool" }>): Item[] {
  const next: ToolItem = {
    kind: "tool",
    id: event.id,
    title: event.title || "工具",
    toolKind: event.kind,
    status: event.status,
    detail: event.detail,
    subagent: event.subagent,
  };
  const index = items.findIndex((item) => item.kind === "tool" && item.id === event.id);
  if (index === -1) {
    return [...items, next];
  }
  const old = items[index] as ToolItem;
  const merged: ToolItem = {
    kind: "tool",
    id: event.id,
    title: event.partial && !event.title ? old.title : event.title || old.title,
    toolKind: event.partial && !event.kind ? old.toolKind : event.kind || old.toolKind,
    status: event.partial && !event.status ? old.status : event.status || old.status,
    detail: event.partial && !event.detail ? old.detail : event.detail || old.detail,
    subagent: event.subagent || old.subagent,
  };
  const copy = items.slice();
  copy[index] = merged;
  return copy;
}

function cancelTools(items: Item[]): Item[] {
  return items.map((item) => {
    if (item.kind === "tool" && (item.status === "pending" || item.status === "in_progress" || item.status === "")) {
      return { ...item, status: "cancelled" };
    }
    return item;
  });
}

export default function Chat({ version }: { version: string | null }) {
  const [cwd, setCwd] = useState("");
  const [sessionId, setSessionId] = useState("");
  const [refreshKey, setRefreshKey] = useState(0);
  const [text, setText] = useState("");
  const [phase, setPhase] = useState<Phase>("idle");
  const [items, setItems] = useState<Item[]>([]);
  const [plan, setPlan] = useState<PlanEntry[] | null>(null);
  const [permission, setPermission] = useState<Extract<AgentEvent, { type: "permission" }> | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const epoch = useRef(0);
  const stopRequested = useRef(false);
  const planRef = useRef<PlanEntry[] | null>(null);
  planRef.current = plan;

  useEffect(() => {
    invoke<{ cwd: string; sessionId: string }>("desk_info")
      .then((info) => {
        setCwd(info.cwd);
        setSessionId(info.sessionId);
      })
      .catch(() => setCwd(""));
  }, []);

  useEffect(() => {
    const unlisten = listen<AgentEvent>("agent-event", (event) => {
      const payload = event.payload;
      if (payload.type === "reset") {
        setItems([]);
        setPlan(null);
        setPermission(null);
        setCwd(payload.cwd);
        setSessionId(payload.sessionId);
        setRefreshKey((value) => value + 1);
        return;
      }
      if (payload.type === "user") {
        setItems((prev) => appendText(prev, "user", payload.text));
        return;
      }
      if (payload.type === "note") {
        setItems((prev) => [...prev, { kind: "note", tone: "plain", text: payload.text }]);
        return;
      }
      if (payload.type === "message") {
        setItems((prev) => appendText(prev, "agent", payload.text));
        return;
      }
      if (payload.type === "thought") {
        setItems((prev) => appendText(prev, "thought", payload.text));
        return;
      }
      if (payload.type === "tool") {
        setItems((prev) => mergeTool(prev, payload));
        return;
      }
      if (payload.type === "plan") {
        setPlan(payload.entries);
        return;
      }
      if (payload.type === "permission") {
        setPermission(payload);
        return;
      }
      if (payload.type === "turn") {
        epoch.current += 1;
        setPhase("idle");
        setPermission(null);
        const note = stopNote(payload.stopReason);
        setItems((prev) => {
          const marked = payload.stopReason === "cancelled" ? cancelTools(prev) : prev;
          return note ? [...marked, { kind: "note", tone: "plain", text: note }] : marked;
        });
        setRefreshKey((value) => value + 1);
        return;
      }
      epoch.current += 1;
      setPhase("idle");
      setPermission(null);
      setItems((prev) => [
        ...prev,
        { kind: "note", tone: "bad", text: `${payload.title}。${payload.detail}` },
      ]);
    });
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [items, plan, permission, phase]);

  async function send() {
    const value = text.trim();
    if (!value || phase !== "idle") return;
    stopRequested.current = false;
    const mine = ++epoch.current;
    const archived = planRef.current;
    setItems((prev) => {
      const next = archived ? [...prev, { kind: "plan" as const, entries: archived }] : prev.slice();
      next.push({ kind: "user", text: value });
      return next;
    });
    setPlan(null);
    setText("");
    setPhase("starting");
    try {
      await invoke("agent_send", { text: value });
      if (epoch.current === mine) setPhase("running");
    } catch (error) {
      if (epoch.current !== mine) return;
      epoch.current += 1;
      setPhase("idle");
      const stopped = stopRequested.current;
      setItems((prev) => [
        ...prev,
        { kind: "note", tone: stopped ? "plain" : "bad", text: stopped ? "已停止。" : errorText(error) },
      ]);
    }
  }

  async function stop() {
    stopRequested.current = true;
    setPermission(null);
    setItems((prev) => cancelTools(prev));
    try {
      await invoke("agent_cancel");
    } catch (error) {
      setItems((prev) => [...prev, { kind: "note", tone: "bad", text: errorText(error) }]);
    }
  }

  async function choose(optionId: string) {
    if (!permission) return;
    const requestId = permission.requestId;
    setPermission(null);
    try {
      await invoke("agent_reply_permission", { requestId, optionId });
    } catch (error) {
      setItems((prev) => [...prev, { kind: "note", tone: "bad", text: errorText(error) }]);
    }
  }

  const busy = phase !== "idle";
  const status = phase === "starting" ? "正在启动命令行…" : phase === "running" ? "正在回复…" : "空闲";

  const liveSubagents: LiveSubagent[] = [];
  const seenSubagents = new Set<string>();
  const liveFiles: string[] = [];
  const liveLinks: string[] = [];
  for (const item of items) {
    if (item.kind === "tool" && item.subagent && !seenSubagents.has(item.id)) {
      seenSubagents.add(item.id);
      liveSubagents.push({ id: item.id, title: item.title, status: item.status, detail: item.detail });
    }
    if (item.kind === "tool") {
      for (const line of item.detail.split("\n")) {
        if (!line.startsWith("改动 ")) continue;
        const path = line.slice("改动 ".length).trim();
        if (path && path !== "文件" && !liveFiles.some((item) => item.toLowerCase() === path.toLowerCase())) {
          liveFiles.push(path);
        }
      }
    }
    if (item.kind === "agent") {
      for (const link of extractHttpUrls(item.text)) {
        if (!liveLinks.includes(link)) liveLinks.push(link);
      }
    }
  }

  async function useProject(next: string) {
    await invoke("agent_set_project", { cwd: next });
  }

  async function newChat() {
    await invoke("agent_new_session");
  }

  async function loadChat(id: string) {
    stopRequested.current = false;
    const mine = ++epoch.current;
    setPhase("starting");
    try {
      await invoke("agent_load_session", { sessionId: id });
      if (epoch.current === mine) setPhase("idle");
    } catch {
      if (epoch.current !== mine) return;
      setPhase("idle");
    }
  }

  return (
    <div className="chat">
      <Side
        cwd={cwd}
        sessionId={sessionId}
        busy={busy}
        refreshKey={refreshKey}
        liveSubagents={liveSubagents}
        liveFiles={liveFiles}
        liveLinks={liveLinks}
        onProject={useProject}
        onNew={newChat}
        onLoad={loadChat}
        onInsert={(snippet) => setText((prev) => (prev.includes(snippet) ? prev : `${prev}${snippet}`))}
      />
      <div className="chat-main">
      <div className="transcript">
        <div className="thread">
          {items.length === 0 && !plan ? (
            <section className="card empty">
              <h2>开始一段对话</h2>
              <p>命令行要等你发出第一条消息才启动。空闲时不占着它。</p>
              <p>执行命令、改文件或联网之前，会先在这里问你。可以随时停止。</p>
              {version ? (
                <p>
                  命令行版本 <span className="path">{version}</span>
                </p>
              ) : null}
              {cwd ? (
                <p>
                  工作目录 <span className="path">{cwd}</span>
                </p>
              ) : null}
              <p>左边可以换文件夹、打开以前的会话，把旧对话归档或删除，也能查看产物、网页、子代理、技能、钩子和工作树。</p>
            </section>
          ) : null}
          {items.map((item, index) => (
            <TranscriptItem key={item.kind === "tool" ? item.id : index} item={item} />
          ))}
          {plan ? <PlanBlock entries={plan} /> : null}
          <div ref={bottomRef} />
        </div>
      </div>
      <form
        className="composer"
        onSubmit={(event) => {
          event.preventDefault();
          if (busy) void stop();
          else void send();
        }}
      >
        <p className="status-line">{status}</p>
        <div className="composer-inner">
          <textarea
            value={text}
            placeholder={busy ? "正在回复，可以点停止" : "输入问题，Enter 发送，Shift+Enter 换行"}
            disabled={busy}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                if (!busy) void send();
              }
            }}
          />
          {busy ? (
            <button type="submit" className="stop">
              停止
            </button>
          ) : (
            <button type="submit" disabled={!text.trim()}>
              发送
            </button>
          )}
        </div>
      </form>
      </div>
      {permission ? (
        <div className="modal-back" role="presentation">
          <section className="modal" role="dialog" aria-modal="true" aria-labelledby="permit-title">
            <h2 id="permit-title">需要你确认</h2>
            <p className="hint">{kindLabel(permission.kind)}</p>
            <p>{permission.title}</p>
            <p className="hint">只在当前这次会话里记住选择。</p>
            <div className="choices">
              {permission.options.map((option) => (
                <button
                  key={option.id}
                  type="button"
                  className={option.kind.startsWith("reject") ? "ghost" : undefined}
                  onClick={() => void choose(option.id)}
                >
                  {optionLabel(option)}
                </button>
              ))}
              <button type="button" className="stop" onClick={() => void stop()}>
                停止
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </div>
  );
}

function TranscriptItem({ item }: { item: Item }) {
  if (item.kind === "user") return <p className="msg user">{item.text}</p>;
  if (item.kind === "agent") return <p className="msg agent">{item.text}</p>;
  if (item.kind === "thought") {
    return (
      <div className="thought">
        <p className="label">思考</p>
        <p>{item.text}</p>
      </div>
    );
  }
  if (item.kind === "tool") {
    return (
      <article className="tool">
        <header>
          <strong>{item.title}</strong>
          <span>
            {kindLabel(item.toolKind)}
            {item.status ? ` · ${statusLabel(item.status)}` : ""}
          </span>
        </header>
        {item.detail ? <pre className="detail">{item.detail}</pre> : null}
      </article>
    );
  }
  if (item.kind === "plan") return <PlanBlock entries={item.entries} />;
  return <p className={item.tone === "bad" ? "note bad" : "note"}>{item.text}</p>;
}

function PlanBlock({ entries }: { entries: PlanEntry[] }) {
  return (
    <section className="plan">
      <p className="label">计划</p>
      <ol>
        {entries.map((entry, index) => (
          <li key={`${entry.status}-${index}`}>
            <span className="plan-status">{statusLabel(entry.status) || "等待"}</span>
            {entry.content}
          </li>
        ))}
      </ol>
    </section>
  );
}

function appendText(items: Item[], kind: "agent" | "thought" | "user", text: string): Item[] {
  const last = items[items.length - 1];
  if (last && last.kind === kind) {
    const copy = items.slice();
    copy[copy.length - 1] = { kind, text: last.text + text };
    return copy;
  }
  return [...items, { kind, text }];
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "操作没有完成。";
}
