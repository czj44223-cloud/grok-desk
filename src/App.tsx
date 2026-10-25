import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import Chat from "./chat";

type Theme = "light" | "dark";

type CliReport = {
  ok: boolean;
  path: string | null;
  version: string | null;
  title: string;
  detail: string;
  steps: string[];
};

function currentTheme(): Theme {
  return document.documentElement.dataset.theme === "dark" ? "dark" : "light";
}

export default function App() {
  const [theme, setTheme] = useState<Theme>(currentTheme);
  const [report, setReport] = useState<CliReport | null>(null);
  const [failed, setFailed] = useState(false);
  const [checking, setChecking] = useState(true);

  const check = useCallback(() => {
    setChecking(true);
    setFailed(false);
    invoke<CliReport>("check_cli")
      .then((next) => {
        setReport(next);
        setChecking(false);
      })
      .catch(() => {
        setReport(null);
        setFailed(true);
        setChecking(false);
      });
  }, []);

  useEffect(() => {
    check();
  }, [check]);

  function toggleTheme() {
    const next: Theme = theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("grok-desk-theme", next);
    setTheme(next);
  }

  return (
    <div className="app">
      <header className="topbar">
        <div>
          <h1>Grok 桌面</h1>
          <p>{report?.ok ? "对话" : "环境检查"}</p>
        </div>
        <div className="topbar-actions">
          {report?.ok ? <LoginCluster /> : null}
          <button type="button" className="ghost" onClick={toggleTheme}>
            {theme === "dark" ? "切换到浅色" : "切换到深色"}
          </button>
        </div>
      </header>
      <main className={report?.ok ? "stage chat-stage" : "stage"}>
        {checking ? <Checking /> : null}
        {!checking && failed ? <InvokeFailed onRetry={check} /> : null}
        {!checking && report && !report.ok ? <ReportCard report={report} onRetry={check} /> : null}
        {!checking && report?.ok ? <Chat version={report.version} /> : null}
      </main>
    </div>
  );
}

type AuthStatus = {
  loggedIn: boolean;
  name: string;
  email: string;
  mode: string;
};

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return "操作没有完成。";
}

function LoginCluster() {
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState("");

  const refresh = useCallback(() => {
    invoke<AuthStatus>("auth_status")
      .then(setStatus)
      .catch(() => setStatus(null));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function login() {
    if (status?.loggedIn && !window.confirm("重新登录会换掉现在的账号。")) return;
    setBusy(true);
    setNote("正在打开浏览器…");
    try {
      const message = await invoke<string>("auth_login");
      setNote(message);
    } catch (error) {
      setNote(errorText(error));
    } finally {
      setBusy(false);
      refresh();
    }
  }

  async function logout() {
    if (!window.confirm("退出登录？命令行会清掉保存的登录。")) return;
    setBusy(true);
    setNote("正在退出…");
    try {
      const message = await invoke<string>("auth_logout");
      setNote(message);
    } catch (error) {
      setNote(errorText(error));
    } finally {
      setBusy(false);
      refresh();
    }
  }

  const label = status?.loggedIn ? status.name || status.email || "已登录" : "未登录";

  return (
    <>
      <span className="login-name">{label}</span>
      {note ? <span className="login-note">{note}</span> : null}
      <button type="button" className="ghost" disabled={busy} onClick={() => void login()}>
        {busy && note.startsWith("正在打开") ? "正在登录…" : status?.loggedIn ? "重新登录" : "登录"}
      </button>
      {status?.loggedIn ? (
        <button type="button" className="ghost" disabled={busy} onClick={() => void logout()}>
          {busy && note.startsWith("正在退出") ? "正在退出…" : "退出"}
        </button>
      ) : null}
    </>
  );
}

function Checking() {
  return (
    <section className="card" aria-live="polite">
      <h2>正在检查本机的 Grok 命令行</h2>
      <p>会运行一次 grok --version，通常一两秒内结束。</p>
    </section>
  );
}

function InvokeFailed({ onRetry }: { onRetry: () => void }) {
  return (
    <section className="card" aria-live="polite">
      <p className="badge bad">检查没有完成</p>
      <h2>界面没能问到命令行状态</h2>
      <p>窗口还在，没有闪退。点下面重试一次。如果每次都这样，关掉窗口再打开。</p>
      <div className="actions">
        <button type="button" onClick={onRetry}>
          重新检查
        </button>
      </div>
    </section>
  );
}

function ReportCard({ report, onRetry }: { report: CliReport; onRetry: () => void }) {
  return (
    <section className="card" aria-live="polite">
      <p className={report.ok ? "badge ok" : "badge bad"}>{report.ok ? "正常" : "需要处理"}</p>
      <h2>{report.title}</h2>
      <p>{report.detail}</p>
      {report.version || report.path ? (
        <dl className="facts">
          {report.version ? (
            <>
              <dt>版本</dt>
              <dd>{report.version}</dd>
            </>
          ) : null}
          {report.path ? (
            <>
              <dt>路径</dt>
              <dd className="path">{report.path}</dd>
            </>
          ) : null}
        </dl>
      ) : null}
      {report.steps.length > 0 ? (
        <ol className="steps">
          {report.steps.map((step) => (
            <li key={step}>{step}</li>
          ))}
        </ol>
      ) : null}
      <div className="actions">
        <button type="button" onClick={onRetry}>
          重新检查
        </button>
      </div>
    </section>
  );
}
