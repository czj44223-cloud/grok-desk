import { useEffect, useMemo, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

type SessionRow = {
  id: string;
  title: string;
  summary: string;
  updatedAt: string;
  model: string;
  archived: boolean;
};

type OutputFile = {
  path: string;
  name: string;
};

type Outputs = {
  files: OutputFile[];
  links: string[];
};

type Preview = {
  path: string;
  name: string;
  text: string;
  binary: boolean;
  truncated: boolean;
};

type SkillRow = {
  name: string;
  description: string;
  scope: string;
  invocable: boolean;
  hint: string;
  path: string;
};

type HookRow = {
  scope: string;
  event: string;
  matcher: string;
  kind: string;
  target: string;
  runs: boolean;
  file: string;
};

type WorktreeRow = {
  id: string;
  path: string;
  label: string;
  branch: string;
  status: string;
  kind: string;
  removable: boolean;
};

type SubagentRow = {
  id: string;
  title: string;
  status: string;
  kind: string;
  isolation: string;
};

type PluginRow = {
  name: string;
  description: string;
  enabled: boolean;
  kind: string;
  toggle: boolean;
};

type PluginCatalog = {
  plugins: PluginRow[];
  note: string;
};

type Catalog = {
  sessions: SessionRow[];
  skills: SkillRow[];
  hooks: HookRow[];
  trustNote: string;
  worktrees: WorktreeRow[];
  worktreeNote: string;
  subagents: SubagentRow[];
  recent: string[];
};

export type LiveSubagent = {
  id: string;
  title: string;
  status: string;
  detail: string;
};

type Props = {
  cwd: string;
  sessionId: string;
  busy: boolean;
  refreshKey: number;
  liveSubagents: LiveSubagent[];
  liveFiles: string[];
  liveLinks: string[];
  onProject: (cwd: string) => Promise<void>;
  onNew: () => Promise<void>;
  onLoad: (sessionId: string) => Promise<void>;
  onInsert: (text: string) => void;
};

const STATUS_LABEL: Record<string, string> = {
  pending: "等待",
  in_progress: "进行中",
  completed: "完成",
  failed: "失败",
  cancelled: "已停止",
  running: "进行中",
};

function statusLabel(status: string): string {
  if (!status) return "";
  return STATUS_LABEL[status] ?? status;
}

function isolationLabel(value: string): string {
  if (value === "worktree") return "独立工作树";
  if (value === "none" || value === "") return "当前目录";
  return value;
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "操作没有完成。";
}

export default function Side({
  cwd,
  sessionId,
  busy,
  refreshKey,
  liveSubagents,
  liveFiles,
  liveLinks,
  onProject,
  onNew,
  onLoad,
  onInsert,
}: Props) {
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [query, setQuery] = useState("");
  const [treeName, setTreeName] = useState("");
  const [note, setNote] = useState("");
  const [noteBad, setNoteBad] = useState(false);
  const [localBusy, setLocalBusy] = useState("");
  const [localTick, setLocalTick] = useState(0);
  const [showArchived, setShowArchived] = useState(false);
  const [outputs, setOutputs] = useState<Outputs>({ files: [], links: [] });
  const [preview, setPreview] = useState<Preview | null>(null);
  const [plugins, setPlugins] = useState<PluginCatalog | null>(null);
  const [pluginTick, setPluginTick] = useState(0);

  useEffect(() => {
    if (!cwd) return;
    let alive = true;
    invoke<Catalog>("project_catalog", { cwd, sessionId })
      .then((next) => {
        if (alive) setCatalog(next);
      })
      .catch((error) => {
        if (!alive) return;
        setNoteBad(true);
        setNote(errorText(error));
      });
    return () => {
      alive = false;
    };
  }, [cwd, sessionId, refreshKey, localTick]);

  useEffect(() => {
    if (!cwd || !sessionId) {
      setOutputs({ files: [], links: [] });
      return;
    }
    let alive = true;
    invoke<Outputs>("session_outputs", { cwd, sessionId })
      .then((next) => {
        if (alive) setOutputs(next);
      })
      .catch(() => {
        if (alive) setOutputs({ files: [], links: [] });
      });
    return () => {
      alive = false;
    };
  }, [cwd, sessionId, refreshKey, localTick]);

  useEffect(() => {
    setPreview(null);
  }, [cwd, sessionId]);

  useEffect(() => {
    let alive = true;
    invoke<PluginCatalog>("plugin_catalog")
      .then((next) => {
        if (alive) setPlugins(next);
      })
      .catch((error) => {
        if (!alive) return;
        setNoteBad(true);
        setNote(errorText(error));
      });
    return () => {
      alive = false;
    };
  }, [pluginTick]);

  useEffect(() => {
    if (!preview) return;
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setPreview(null);
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [preview]);

  const archivedCount = (catalog?.sessions ?? []).filter((row) => row.archived).length;

  const sessions = useMemo(() => {
    const q = query.trim().toLowerCase();
    const rows = catalog?.sessions ?? [];
    return rows.filter((row) => {
      const shown = showArchived ? row.archived : !row.archived || row.id === sessionId;
      if (!shown) return false;
      if (!q) return true;
      return `${row.title} ${row.summary}`.toLowerCase().includes(q);
    });
  }, [catalog, query, showArchived, sessionId]);

  const products = useMemo(() => {
    const files = outputs.files.slice();
    for (const path of liveFiles) {
      if (!files.some((file) => file.path.toLowerCase() === path.toLowerCase())) {
        const parts = path.split(/[/\\]/);
        files.push({ path, name: parts[parts.length - 1] || path });
      }
    }
    const links = outputs.links.slice();
    for (const link of liveLinks) {
      if (!links.includes(link)) links.push(link);
    }
    return { files: files.slice(0, 40), links: links.slice(0, 40) };
  }, [outputs, liveFiles, liveLinks]);

  const subagents = useMemo(() => {
    const disk = catalog?.subagents ?? [];
    const seen = new Set(liveSubagents.map((item) => item.id));
    const live = liveSubagents.map((item) => ({
      id: item.id,
      title: item.title || "子代理",
      status: statusLabel(item.status),
      extra: item.detail,
    }));
    const rest = disk
      .filter((item) => !seen.has(item.id))
      .map((item) => ({
        id: item.id,
        title: item.title,
        status: statusLabel(item.status),
        extra: [item.kind && `类型 ${item.kind}`, isolationLabel(item.isolation)].filter(Boolean).join(" · "),
      }));
    return [...live, ...rest];
  }, [catalog, liveSubagents]);

  async function run(label: string, action: () => Promise<void>) {
    setNote("");
    setNoteBad(false);
    setLocalBusy(label);
    try {
      await action();
    } catch (error) {
      setNoteBad(true);
      setNote(errorText(error));
    } finally {
      setLocalBusy("");
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, multiple: false, title: "选择文件夹" });
    if (typeof picked !== "string" || !picked) return;
    await onProject(picked);
  }

  async function createTree(event: FormEvent) {
    event.preventDefault();
    const name = treeName.trim();
    if (!name) return;
    const ok = window.confirm(`新建工作树「${name}」。会从当前提交复制未提交的改动。`);
    if (!ok) return;
    await run("正在新建工作树…", async () => {
      const path = await invoke<string>("create_worktree", { cwd, name });
      setTreeName("");
      setNote(path ? `已新建：${path}` : "已新建工作树。");
      setLocalTick((value) => value + 1);
    });
  }

  async function changeArchive(row: SessionRow, archived: boolean) {
    await run(archived ? "正在归档…" : "正在取消归档…", async () => {
      await invoke("set_session_archived", { cwd, sessionId: row.id, archived });
      setNote(archived ? "已归档。可以在「已归档」里找回来。" : "已取消归档。");
      setLocalTick((value) => value + 1);
    });
  }

  async function removeSession(row: SessionRow) {
    const ok = window.confirm("永久删除这个会话？命令行会把它从历史里去掉，不能恢复。");
    if (!ok) return;
    await run("正在删除会话…", async () => {
      await invoke("delete_session", { cwd, sessionId: row.id });
      setNote("已删除。");
      setLocalTick((value) => value + 1);
    });
  }

  async function viewFile(path: string) {
    await run("正在读取文件…", async () => {
      const next = await invoke<Preview>("preview_file", { path });
      setPreview(next);
    });
  }

  async function openFile(path: string) {
    await run("正在打开文件…", async () => {
      await invoke("open_path", { path });
      setNote("已交给系统打开。");
    });
  }

  async function openLink(url: string) {
    await run("正在打开网页…", async () => {
      await invoke("open_url", { url });
      setNote("已在浏览器打开。");
    });
  }

  async function togglePlugin(row: PluginRow) {
    if (!row.toggle) return;
    const next = !row.enabled;
    if (row.name === "computer-use" && next) {
      const ok = window.confirm("打开电脑操作？模型将可以截屏、点击和打字。每次操作仍会先问你。");
      if (!ok) return;
    }
    await run(next ? "正在打开插件…" : "正在关闭插件…", async () => {
      const message = await invoke<string>("set_plugin_enabled", { name: row.name, enabled: next });
      setNote(message);
      setPluginTick((value) => value + 1);
    });
  }

  async function removeTree(row: WorktreeRow) {
    const ok = window.confirm(`移除工作树「${row.label}」？没保存的改动会留在原地，命令行不会强制删除。`);
    if (!ok) return;
    await run("正在移除工作树…", async () => {
      await invoke("remove_worktree", { cwd, id: row.id, path: row.path, kind: row.kind });
      setNote("已移除工作树。");
      setLocalTick((value) => value + 1);
    });
  }

  const recent = (catalog?.recent ?? []).filter((path) => path.toLowerCase() !== cwd.toLowerCase());

  return (
    <aside className="side">
      <p className="side-label">文件夹</p>
      <p className="path">{cwd || "还没有文件夹"}</p>
      <div className="side-actions">
        <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void run("正在切换文件夹…", pickFolder)}>
          打开文件夹
        </button>
        <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void run("正在新开会话…", onNew)}>
          新对话
        </button>
      </div>
      {recent.length > 0 ? (
        <div className="recent">
          {recent.map((path) => (
            <button key={path} type="button" className="row" disabled={busy || !!localBusy} onClick={() => void run("正在切换文件夹…", () => onProject(path))}>
              <span className="row-title">{path}</span>
            </button>
          ))}
        </div>
      ) : null}

      <details open>
        <summary>插件 {plugins?.plugins.length ?? 0}</summary>
        <p className="hint">{plugins?.note || "正在读取插件…"}</p>
        {(plugins?.plugins ?? []).map((row) => (
          <div key={row.name} className="block">
            <strong>{row.name === "computer-use" ? "电脑操作" : row.name}</strong>
            <span className="row-meta">
              {" "}
              {row.kind}
              {row.enabled ? " · 已打开" : " · 已关闭"}
            </span>
            {row.description ? <p className="clamp">{row.description}</p> : null}
            {row.toggle ? (
              <div className="side-actions">
                <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void togglePlugin(row)}>
                  {row.enabled ? "关闭" : "打开"}
                </button>
              </div>
            ) : (
              <p className="hint">这个名字不能从这里开关。</p>
            )}
          </div>
        ))}
      </details>

      <div className="side-head">
        <p className="side-label">会话</p>
        <button
          type="button"
          className={showArchived ? "ghost pressed" : "ghost"}
          onClick={() => setShowArchived((value) => !value)}
        >
          {showArchived ? "返回列表" : `已归档${archivedCount > 0 ? ` ${archivedCount}` : ""}`}
        </button>
      </div>
      <input
        value={query}
        placeholder="搜索最近的会话"
        onChange={(event) => setQuery(event.target.value)}
      />
      <div className="session-list">
        {sessions.length === 0 ? (
          <p className="hint">
            {showArchived ? "没有归档的会话。" : archivedCount > 0 ? "剩下的会话都已归档。" : "这个文件夹还没有会话。"}
          </p>
        ) : null}
        {sessions.map((row) => (
          <div key={row.id} className={row.id === sessionId ? "session current" : "session"}>
            <button
              type="button"
              className="row-open"
              disabled={busy || !!localBusy}
              onClick={() => void run("正在打开会话…", () => onLoad(row.id))}
            >
              <span className="row-title">{row.title}</span>
              <span className="row-meta">
                {row.updatedAt.slice(0, 10)}
                {row.model ? ` · ${row.model}` : ""}
                {row.archived ? " · 已归档" : ""}
              </span>
            </button>
            <div className="side-actions">
              {row.archived ? (
                <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void changeArchive(row, false)}>
                  取消归档
                </button>
              ) : (
                <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void changeArchive(row, true)}>
                  归档
                </button>
              )}
              <button type="button" className="danger" disabled={busy || !!localBusy} onClick={() => void removeSession(row)}>
                删除
              </button>
            </div>
          </div>
        ))}
      </div>

      <details>
        <summary>产物 {products.files.length + products.links.length}</summary>
        {!sessionId ? <p className="hint">打开一个会话后，这里列出它改过的文件和回复里的网页。</p> : null}
        {sessionId && products.files.length === 0 && products.links.length === 0 ? (
          <p className="hint">这次会话还没有改过文件，回复里也没有网页。</p>
        ) : null}
        {products.files.map((file) => (
          <div key={file.path} className="block">
            <strong>{file.name}</strong>
            <p className="clamp path">{file.path}</p>
            <div className="side-actions">
              <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void viewFile(file.path)}>
                查看
              </button>
              <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void openFile(file.path)}>
                用系统程序打开
              </button>
            </div>
          </div>
        ))}
        {products.links.map((link) => (
          <div key={link} className="block">
            <p className="clamp path">{link}</p>
            <div className="side-actions">
              <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void openLink(link)}>
                打开网页
              </button>
            </div>
          </div>
        ))}
      </details>

      <details>
        <summary>子代理 {subagents.length}</summary>
        {subagents.length === 0 ? <p className="hint">这一会话还没有子代理。模型要用时，由命令行启动。</p> : null}
        {subagents.map((row) => (
          <div key={row.id} className="block">
            <strong>{row.title}</strong>
            {row.status ? <span className="row-meta">{row.status}</span> : null}
            {row.extra ? <p className="clamp">{row.extra}</p> : null}
          </div>
        ))}
      </details>

      <details>
        <summary>技能 {catalog?.skills.length ?? 0}</summary>
        <p className="hint">点一下，把斜杠命令放进输入框。真正执行的是命令行。</p>
        {(catalog?.skills ?? []).map((skill) =>
          skill.invocable ? (
            <button key={skill.path} type="button" className="row" onClick={() => onInsert(`/${skill.name} `)}>
              <span className="row-title">
                /{skill.name}
                <span className="row-meta"> {skill.scope}</span>
              </span>
              {skill.description ? <span className="clamp">{skill.description}</span> : null}
            </button>
          ) : (
            <div key={skill.path} className="block">
              <strong>/{skill.name}</strong>
              <span className="row-meta"> {skill.scope} · 仅模型调用</span>
              {skill.description ? <p className="clamp">{skill.description}</p> : null}
            </div>
          ),
        )}
      </details>

      <details>
        <summary>钩子 {catalog?.hooks.length ?? 0}</summary>
        {catalog?.trustNote ? <p className="hint">{catalog.trustNote}</p> : null}
        {(catalog?.hooks.length ?? 0) === 0 ? <p className="hint">钩子目录里没有 json 文件。配置文件里的钩子仍由命令行读取。</p> : null}
        {(catalog?.hooks ?? []).map((hook, index) => (
          <div key={`${hook.file}-${hook.event}-${index}`} className="block">
            <strong>{hook.event}</strong>
            <span className="row-meta">
              {" "}
              {hook.scope}
              {hook.matcher ? ` · ${hook.matcher}` : ""}
              {hook.runs ? "" : " · 这种写法命令行目前不会执行"}
            </span>
            {hook.target ? <p className="clamp">{hook.target}</p> : null}
          </div>
        ))}
      </details>

      <details>
        <summary>工作树 {catalog?.worktrees.length ?? 0}</summary>
        <p className="hint">新建和移除都调用本机的 grok worktree。在此打开会换到那个目录，并开始新对话。</p>
        {catalog?.worktreeNote ? <p className="hint">{catalog.worktreeNote}</p> : null}
        {(catalog?.worktrees ?? []).map((row) => (
          <div key={`${row.kind}-${row.path}`} className="block">
            <strong>{row.label}</strong>
            <span className="row-meta">
              {" "}
              {row.branch}
              {row.status ? ` · ${row.status}` : ""}
            </span>
            <p className="path">{row.path}</p>
            <div className="side-actions">
              {row.path.toLowerCase() !== cwd.toLowerCase() ? (
                <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void run("正在打开工作树…", () => onProject(row.path))}>
                  在此打开
                </button>
              ) : null}
              {row.removable ? (
                <button type="button" className="ghost" disabled={busy || !!localBusy} onClick={() => void removeTree(row)}>
                  移除
                </button>
              ) : null}
            </div>
          </div>
        ))}
        <form className="tree-form" onSubmit={(event) => void createTree(event)}>
          <input value={treeName} placeholder="工作树名字" onChange={(event) => setTreeName(event.target.value)} />
          <button type="submit" className="ghost" disabled={busy || !!localBusy || !treeName.trim()}>
            新建
          </button>
        </form>
      </details>

      {localBusy ? <p className="hint">{localBusy}</p> : null}
      {note ? <p className={noteBad ? "side-note bad" : "side-note"}>{note}</p> : null}
      {preview ? (
        <div className="modal-back" onClick={() => setPreview(null)}>
          <div
            className="modal preview"
            role="dialog"
            aria-modal="true"
            aria-label={preview.name}
            onClick={(event) => event.stopPropagation()}
          >
            <h2>{preview.name}</h2>
            {preview.binary ? <p>这是二进制文件，请用系统程序打开。</p> : <pre className="preview-text">{preview.text}</pre>}
            {preview.truncated ? <p className="hint">只显示前 32 KB。</p> : null}
            <div className="choices">
              <button type="button" className="ghost" onClick={() => setPreview(null)}>
                关闭
              </button>
              <button type="button" className="ghost" onClick={() => void openFile(preview.path)}>
                用系统程序打开
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </aside>
  );
}
