# Grok 桌面

给不想用终端的人用的 Grok 窗口。程序自己不回答问题，后台调用本机的 `grok` 命令行。

当前可以：打开一个会话，流式看到回复、思考和工具调用，执行前询问，也可以停止。左边能换文件夹、搜索并打开这个文件夹里已有的会话。旧会话可以归档或永久删除：归档只记在本机，可以取消；删除调用 `grok sessions delete`。打开会话后，产物列出它改过的文件和回复里的网页，可以预览、用系统程序打开，或在浏览器里打开链接。子代理、技能、钩子和工作树都由本机命令行负责：桌面端列出它们，技能可以放进输入框，工作树的新建和移除调用 `grok worktree`。顶栏可以登录和退出，调用本机的 `grok login` 与 `grok logout`。左边可以开关插件；电脑操作默认关闭，只在这个窗口打开，截屏、点击和打字仍会先询问。还不能按文件审阅改动，也没有内置终端和安装包。

对话由本机的 `grok agent --no-leader stdio` 完成。桌面端只收发 JSON-RPC，不自己实现模型、子代理、钩子或技能。空闲时不启动对话进程。第一次打开时，工作目录是你的用户文件夹。

## 开发

需要：

- Node.js
- Rust（rustup，stable）
- Windows 上还需要 Visual Studio 的「使用 C++ 的桌面开发」工作负载，或 Build Tools 里的 MSVC
- 本机已安装 Grok Build 1.0.70 或更高（`grok --version`）

```powershell
cd grok-desk
npm install
```

在 Windows 上，Rust 要能找到 MSVC 的链接器。直接跑 `npm run tauri dev` 时，如果提示找不到 `link.exe` 或 `dbghelp.lib`，改用下面这个脚本。它会先加载 Visual Studio 的编译环境，再启动窗口：

```powershell
.\scripts\windows-dev.bat
```

macOS 和 Linux 以后可以直接 `npm run tauri dev`。

版本检查的逻辑在 `src-tauri/src/cli.rs`。只跑这块测试：

```powershell
.\scripts\windows-test.bat
```

## 打包

Windows 安装包在最后阶段再打。配置里的目标是 NSIS 的 `.exe`：

```powershell
npm run tauri build
```

装好的程序不包含 Rust。用户机器需要 WebView2（Windows 11 一般已经有）和 Grok 命令行。

## 许可证

MIT，见 [LICENSE](LICENSE)。

---

## English

GrokDesk is a small Tauri 2 + React desktop window for people who prefer not to use a terminal. It does not answer questions itself: it drives the locally installed `grok` CLI (Grok Build 1.0.70+) via `grok agent --no-leader stdio` over JSON-RPC, showing streamed replies, thinking, tool calls with approval prompts, sessions per folder (search / archive / delete), outputs preview, sub-agents, skills, hooks, worktrees, login/logout, and an opt-in computer-use plugin.

Dev: Node.js, Rust (stable), and on Windows the MSVC "Desktop development with C++" workload. `npm install`, then `npm run tauri dev` (or `scripts\windows-dev.bat` on Windows). Build: `npm run tauri build` (NSIS installer). License: MIT.
