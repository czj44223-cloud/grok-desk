import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const root = path.dirname(fileURLToPath(import.meta.url));
const actions = path.join(root, "actions.ps1");
const shell = process.env.GROK_DESK_PWSH || "powershell.exe";

const tools = [
  {
    name: "computer_screenshot",
    description: "截取当前屏幕并保存成图片。返回图片路径、原点和缩放，坐标换算见插件说明。",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "computer_click",
    description: "在屏幕坐标点击。button 只能是 left、right 或 middle。clicks 为 1 或 2。",
    inputSchema: {
      type: "object",
      properties: {
        x: { type: "number" },
        y: { type: "number" },
        button: { type: "string" },
        clicks: { type: "integer" },
      },
      required: ["x", "y"],
      additionalProperties: false,
    },
  },
  {
    name: "computer_type",
    description: "向当前焦点输入文字。一次最多 2000 个字符。换行请改用 computer_key 的 enter。",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string" } },
      required: ["text"],
      additionalProperties: false,
    },
  },
  {
    name: "computer_key",
    description: "按下允许的按键。key 如 enter、tab、escape、backspace、方向键。modifiers 只能包含 ctrl、alt、shift。",
    inputSchema: {
      type: "object",
      properties: {
        key: { type: "string" },
        modifiers: { type: "array", items: { type: "string" } },
      },
      required: ["key"],
      additionalProperties: false,
    },
  },
  {
    name: "computer_scroll",
    description: "在屏幕坐标处滚动。dy 为正向上，为负向下，范围 -10 到 10。",
    inputSchema: {
      type: "object",
      properties: {
        x: { type: "number" },
        y: { type: "number" },
        dy: { type: "integer" },
      },
      required: ["x", "y", "dy"],
      additionalProperties: false,
    },
  },
  {
    name: "computer_windows",
    description: "列出带标题的窗口，用来确认当前焦点。",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
];

const errors = {
  OUT_OF_SCREEN: "这个坐标不在屏幕里。",
  BAD_INPUT: "参数不对。",
  SHOT_FAILED: "截屏没有完成。",
  TYPE_TOO_LONG: "一次最多输入 2000 个字符。",
  TYPE_EMPTY: "没有要输入的文字。",
  UNKNOWN_KEY: "不支持这个按键。",
  UNKNOWN_ACTION: "没有这个操作。",
  NO_SCREEN: "没有找到屏幕。",
  NODE_MISSING: "没有找到 Node.js。",
};

let buffer = Buffer.alloc(0);
let framing = "line";

function send(message) {
  const json = JSON.stringify(message);
  if (framing === "headers") {
    const body = Buffer.from(json, "utf8");
    process.stdout.write(`Content-Length: ${body.length}\r\n\r\n`);
    process.stdout.write(body);
    return;
  }
  process.stdout.write(`${json}\n`);
}

function reply(id, result) {
  send({ jsonrpc: "2.0", id, result });
}

function fail(id, code, message) {
  send({ jsonrpc: "2.0", id, error: { code, message } });
}

function runAction(body) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      shell,
      ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", actions],
      { windowsHide: true, stdio: ["pipe", "pipe", "pipe"] },
    );
    let out = "";
    let err = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      out += chunk;
    });
    child.stderr.on("data", (chunk) => {
      err += chunk;
    });
    child.on("error", () => reject(new Error("BAD_INPUT")));
    child.on("close", (code) => {
      const text = out.trim();
      if (code !== 0) {
        const codeName = (err || text).trim().split(/\s+/)[0] || "BAD_INPUT";
        reject(new Error(codeName));
        return;
      }
      try {
        resolve(JSON.parse(text));
      } catch {
        reject(new Error("BAD_INPUT"));
      }
    });
    child.stdin.write(JSON.stringify(body));
    child.stdin.end();
  });
}

function explain(error) {
  const code = String(error && error.message ? error.message : error).trim();
  return errors[code] || "操作没有完成。";
}

function screenshotText(result) {
  return [
    `图片 ${result.path}`,
    `原点 ${result.originX}, ${result.originY}`,
    `图片大小 ${result.imageWidth} x ${result.imageHeight}`,
    `屏幕大小 ${result.screenWidth} x ${result.screenHeight}`,
    `缩放 ${result.scaleX}, ${result.scaleY}`,
    "屏幕坐标 = 原点 + 图片坐标 × 缩放。",
  ].join("\n");
}

async function callTool(name, args) {
  const input = args && typeof args === "object" ? args : {};
  if (name === "computer_screenshot") {
    const result = await runAction({ action: "screenshot" });
    return screenshotText(result);
  }
  if (name === "computer_click") {
    await runAction({
      action: "click",
      x: input.x,
      y: input.y,
      button: input.button || "left",
      clicks: input.clicks || 1,
    });
    return "已点击。";
  }
  if (name === "computer_type") {
    const result = await runAction({ action: "type", text: input.text || "" });
    return `已输入 ${result.count} 个字符。`;
  }
  if (name === "computer_key") {
    await runAction({ action: "key", key: input.key || "", modifiers: input.modifiers || [] });
    return "已按键。";
  }
  if (name === "computer_scroll") {
    await runAction({ action: "scroll", x: input.x, y: input.y, dy: input.dy });
    return "已滚动。";
  }
  if (name === "computer_windows") {
    const result = await runAction({ action: "windows" });
    if (!result.windows || result.windows.length === 0) return "没有带标题的窗口。";
    return result.windows.map((item) => `${item.title} (${item.process})`).join("\n");
  }
  throw new Error("UNKNOWN_ACTION");
}

async function handle(message) {
  if (!message || message.jsonrpc !== "2.0") return;
  const { id, method, params } = message;
  if (id === undefined) return;
  if (method === "initialize") {
    const requested = params && params.protocolVersion ? params.protocolVersion : "2024-11-05";
    reply(id, {
      protocolVersion: requested,
      capabilities: { tools: {} },
      serverInfo: { name: "computer-use", version: "0.1.0" },
    });
    return;
  }
  if (method === "ping") {
    reply(id, {});
    return;
  }
  if (method === "tools/list") {
    reply(id, { tools });
    return;
  }
  if (method === "tools/call") {
    const name = params && params.name;
    const args = params && params.arguments;
    try {
      const text = await callTool(name, args);
      reply(id, { content: [{ type: "text", text }], isError: false });
    } catch (error) {
      reply(id, { content: [{ type: "text", text: explain(error) }], isError: true });
    }
    return;
  }
  fail(id, -32601, "没有这个方法。");
}

function takeLine() {
  const newline = buffer.indexOf(0x0a);
  if (newline < 0) return null;
  const line = buffer.slice(0, newline).toString("utf8").replace(/\r$/, "");
  buffer = buffer.slice(newline + 1);
  return line.trim() ? line.trim() : "";
}

function takeHeaders() {
  const end = buffer.indexOf("\r\n\r\n");
  if (end < 0) return null;
  const header = buffer.slice(0, end).toString("utf8");
  const match = header.match(/Content-Length:\s*(\d+)/i);
  if (!match) {
    buffer = buffer.slice(end + 4);
    return "";
  }
  const length = Number(match[1]);
  const start = end + 4;
  if (buffer.length < start + length) return null;
  const body = buffer.slice(start, start + length).toString("utf8");
  buffer = buffer.slice(start + length);
  framing = "headers";
  return body;
}

function drain() {
  while (buffer.length > 0) {
    const head = buffer.slice(0, 20).toString("utf8");
    const body = head.startsWith("Content-Length") ? takeHeaders() : takeLine();
    if (body === null) return;
    if (!body) continue;
    let message = null;
    try {
      message = JSON.parse(body);
    } catch {
      continue;
    }
    void handle(message);
  }
}

process.stdin.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  drain();
});
process.stdin.on("end", () => {
  process.exit(0);
});
process.stdin.resume();
