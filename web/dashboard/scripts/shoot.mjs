// Deterministic multi-viewport screenshot tool for static HTML mockups and the
// live dashboard. Reuses the headless-Chrome-over-CDP approach proven in
// browser-smoke.mjs so it stays dependency-free (no Playwright/npm install).
//
// Usage:
//   node web/dashboard/scripts/shoot.mjs --out <dir> <file-or-url> [<file-or-url> ...]
//   node web/dashboard/scripts/shoot.mjs --out <dir> --dir <htmlDir>
//
// For each input it captures a desktop (1440w) and mobile (390w) full-page PNG:
//   <out>/<name>.desktop.png
//   <out>/<name>.mobile.png
//
// Inputs may be local .html paths (rendered via file://) or http(s) URLs.

import { mkdir, writeFile, readdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, basename, extname } from "node:path";
import { pathToFileURL } from "node:url";
import { spawn } from "node:child_process";

const chromePath =
  process.env.CHROME_BIN ||
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const cdpPort = Number.parseInt(process.env.CAPO_SHOOT_CDP_PORT || "9224", 10);

const VIEWPORTS = [
  { name: "desktop", width: 1440, height: 900, scale: 1, mobile: false },
  { name: "mobile", width: 390, height: 844, scale: 2, mobile: true }
];

function parseArgs(argv) {
  const opts = { out: "design-shots", inputs: [], dir: null };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--out") {
      opts.out = argv[++i];
    } else if (arg === "--dir") {
      opts.dir = argv[++i];
    } else {
      opts.inputs.push(arg);
    }
  }
  return opts;
}

function toUrl(input) {
  if (/^https?:\/\//.test(input)) {
    return input;
  }
  return pathToFileURL(resolve(input)).href;
}

function nameFor(input) {
  if (/^https?:\/\//.test(input)) {
    const u = new URL(input);
    const tail =
      u.searchParams.get("name") ||
      u.searchParams.get("theme") ||
      u.searchParams.get("qa") ||
      basename(u.pathname) ||
      u.hostname;
    return (tail || "page").replace(/[^a-z0-9-]+/gi, "-").replace(/^-|-$/g, "");
  }
  return basename(input, extname(input));
}

async function waitForJson(url, timeoutMs = 10000) {
  const start = Date.now();
  let lastError;
  while (Date.now() - start < timeoutMs) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return response.json();
      }
    } catch (error) {
      lastError = error;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw lastError || new Error(`timed out waiting for ${url}`);
}

function connect(wsUrl) {
  const socket = new WebSocket(wsUrl);
  let nextId = 1;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve: res, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        reject(new Error(message.error.message));
      } else {
        res(message.result || {});
      }
    }
  });
  return new Promise((res, rej) => {
    socket.addEventListener("open", () =>
      res({
        send(method, params = {}) {
          const id = nextId++;
          socket.send(JSON.stringify({ id, method, params }));
          return new Promise((resolveSend, rejectSend) => {
            pending.set(id, { resolve: resolveSend, reject: rejectSend });
          });
        },
        close() {
          socket.close();
        }
      })
    );
    socket.addEventListener("error", rej);
  });
}

async function evaluate(client, expression) {
  const result = await client.send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.text);
  }
  return result.result?.value;
}

async function capture(client, url, viewport, outPath) {
  await client.send("Emulation.setDeviceMetricsOverride", {
    width: viewport.width,
    height: viewport.height,
    deviceScaleFactor: viewport.scale,
    mobile: viewport.mobile
  });
  const navUrl = viewport.mobile && url.startsWith("http")
    ? `${url}${url.includes("?") ? "&" : "?"}qa=mobile`
    : url;
  await client.send("Page.navigate", { url: navUrl });
  // Wait for the document to finish loading, then a short beat for layout/fonts.
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const ready = await evaluate(client, "document.readyState === 'complete'");
    if (ready) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  await new Promise((r) => setTimeout(r, 450));
  const result = await client.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: true
  });
  await writeFile(outPath, Buffer.from(result.data, "base64"));
  return outPath;
}

const opts = parseArgs(process.argv.slice(2));
let inputs = [...opts.inputs];
if (opts.dir) {
  const entries = await readdir(opts.dir);
  inputs.push(
    ...entries
      .filter((f) => f.endsWith(".html"))
      .map((f) => join(opts.dir, f))
  );
}
if (inputs.length === 0) {
  console.error("no inputs: pass html files/urls or --dir <htmlDir>");
  process.exit(1);
}

await mkdir(opts.out, { recursive: true });
const userDataDir = await mkdtemp(join(tmpdir(), "capo-shoot-chrome-"));
const chrome = spawn(
  chromePath,
  [
    "--headless=new",
    "--no-first-run",
    "--disable-gpu",
    "--disable-extensions",
    "--hide-scrollbars",
    "--force-color-profile=srgb",
    `--remote-debugging-port=${cdpPort}`,
    `--user-data-dir=${userDataDir}`,
    "about:blank"
  ],
  { stdio: ["ignore", "ignore", "pipe"] }
);

const written = [];
try {
  await waitForJson(`http://127.0.0.1:${cdpPort}/json/version`);
  const target = await fetch(
    `http://127.0.0.1:${cdpPort}/json/new?${encodeURIComponent("about:blank")}`,
    { method: "PUT" }
  ).then((r) => r.json());
  const client = await connect(target.webSocketDebuggerUrl);
  await client.send("Page.enable");
  await client.send("Runtime.enable");

  for (const input of inputs) {
    const url = toUrl(input);
    const name = nameFor(input);
    for (const viewport of VIEWPORTS) {
      const outPath = join(opts.out, `${name}.${viewport.name}.png`);
      await capture(client, url, viewport, outPath);
      written.push(outPath);
      console.log(`shot=${outPath}`);
    }
  }
  client.close();
} finally {
  chrome.kill("SIGTERM");
}

console.log(`done count=${written.length}`);
