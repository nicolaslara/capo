import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "node:child_process";

const chromePath =
  process.env.CHROME_BIN || "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const baseUrl = process.env.CAPO_DASHBOARD_URL || "http://127.0.0.1:4173";
const screenshotDir =
  process.env.CAPO_DASHBOARD_SCREENSHOTS || "workpads/dashboard-webclient/screenshots";
const port = Number.parseInt(process.env.CAPO_DASHBOARD_CDP_PORT || "9223", 10);

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
    await new Promise((resolve) => setTimeout(resolve, 100));
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
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        reject(new Error(message.error.message));
      } else {
        resolve(message.result || {});
      }
    }
  });
  return new Promise((resolve, reject) => {
    socket.addEventListener("open", () => {
      resolve({
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
      });
    });
    socket.addEventListener("error", reject);
  });
}

async function capture(client, fileName) {
  const result = await client.send("Page.captureScreenshot", {
    format: "png",
    captureBeyondViewport: true
  });
  const filePath = join(screenshotDir, fileName);
  await writeFile(filePath, Buffer.from(result.data, "base64"));
  return filePath;
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

async function waitForApp(client) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const ready = await evaluate(
      client,
      "Boolean(document.querySelector('[data-agent=\"codex-local\"]') && document.querySelector('#metric-agents')?.textContent !== '0')"
    );
    if (ready) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("dashboard did not render agent rows");
}

await mkdir(screenshotDir, { recursive: true });
const userDataDir = await mkdtemp(join(tmpdir(), "capo-dashboard-chrome-"));
const chrome = spawn(
  chromePath,
  [
    "--headless=new",
    "--no-first-run",
    "--disable-gpu",
    "--disable-extensions",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${userDataDir}`,
    "about:blank"
  ],
  { stdio: ["ignore", "ignore", "pipe"] }
);

try {
  await waitForJson(`http://127.0.0.1:${port}/json/version`);
  const target = await fetch(`http://127.0.0.1:${port}/json/new?${encodeURIComponent(baseUrl)}`, {
    method: "PUT"
  }).then((response) => response.json());
  const client = await connect(target.webSocketDebuggerUrl);
  await client.send("Page.enable");
  await client.send("Runtime.enable");

  await client.send("Emulation.setDeviceMetricsOverride", {
    width: 1280,
    height: 920,
    deviceScaleFactor: 1,
    mobile: false
  });
  await client.send("Page.navigate", { url: baseUrl });
  await new Promise((resolve) => setTimeout(resolve, 800));
  await waitForApp(client);
  const desktopPath = await capture(client, "dashboard-desktop.png");

  await evaluate(
    client,
    "document.querySelector('[data-agent=\"codex-local\"]').click(); document.querySelector('#steer-input').value = 'Please summarize status and evidence'; document.querySelector('#steer-button').click();"
  );
  await new Promise((resolve) => setTimeout(resolve, 300));
  const commandText = await evaluate(client, "document.querySelector('#command-log').textContent");
  if (!commandText.includes("steer_agent queued for codex-local")) {
    throw new Error(`unexpected command log: ${commandText}`);
  }

  await evaluate(client, "document.querySelector('#debug-toggle').click()");
  const detailsVisible = await evaluate(client, "!document.querySelector('#debug-drawer').hidden");
  if (!detailsVisible) {
    throw new Error("debug drawer did not open");
  }
  const detailPath = await capture(client, "dashboard-detail-command.png");

  await evaluate(client, "document.querySelector('#debug-close').click(); document.querySelector('[data-view=\"goals\"]').click();");
  await new Promise((resolve) => setTimeout(resolve, 200));
  const goalsVisible = await evaluate(client, "!document.querySelector('#goals-view').classList.contains('hidden')");
  if (!goalsVisible) {
    throw new Error("goals view did not open");
  }
  const goalsPath = await capture(client, "dashboard-goals.png");

  await client.send("Emulation.setDeviceMetricsOverride", {
    width: 390,
    height: 980,
    deviceScaleFactor: 2,
    mobile: true
  });
  await client.send("Page.navigate", { url: `${baseUrl}?qa=mobile` });
  await new Promise((resolve) => setTimeout(resolve, 800));
  await waitForApp(client);
  const mobilePath = await capture(client, "dashboard-mobile.png");

  client.close();
  console.log(`desktop=${desktopPath}`);
  console.log(`detail=${detailPath}`);
  console.log(`goals=${goalsPath}`);
  console.log(`mobile=${mobilePath}`);
} finally {
  chrome.kill("SIGTERM");
}
