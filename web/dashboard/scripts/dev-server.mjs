import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL("..", import.meta.url)));
const fixturePath = join(root, "fixtures", "dashboard.json");
const commands = [];

const types = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8"
};

function send(response, status, body, type = "application/json; charset=utf-8") {
  response.writeHead(status, {
    "content-type": type,
    "cache-control": "no-store",
    "access-control-allow-origin": "*",
    "access-control-allow-methods": "GET,POST,OPTIONS",
    "access-control-allow-headers": "content-type"
  });
  response.end(body);
}

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
}

async function serveStatic(pathname, response) {
  const requested = pathname === "/" ? "/index.html" : pathname;
  const safePath = normalize(requested).replace(/^(\.\.[/\\])+/, "");
  const filePath = join(root, safePath);
  if (!filePath.startsWith(root)) {
    send(response, 403, "forbidden", "text/plain; charset=utf-8");
    return;
  }
  try {
    const body = await readFile(filePath);
    send(response, 200, body, types[extname(filePath)] || "application/octet-stream");
  } catch {
    send(response, 404, "not found", "text/plain; charset=utf-8");
  }
}

const server = createServer(async (request, response) => {
  try {
    if (request.method === "OPTIONS") {
      send(response, 204, "");
      return;
    }
    const url = new URL(request.url || "/", `http://${request.headers.host}`);
    if (request.method === "GET" && url.pathname === "/api/dashboard") {
      send(response, 200, await readFile(fixturePath, "utf8"));
      return;
    }
    if (request.method === "GET" && url.pathname === "/api/commands") {
      send(response, 200, JSON.stringify({ commands }));
      return;
    }
    if (request.method === "POST" && url.pathname === "/api/commands") {
      const body = await readJson(request);
      const allowed = new Set(["steer_agent", "interrupt_agent", "stop_agent"]);
      if (!allowed.has(body.kind) || !body.agent) {
        send(response, 400, JSON.stringify({ error: "unsupported command" }));
        return;
      }
      const command = {
        id: `mock-command-${commands.length + 1}`,
        kind: body.kind,
        agent: body.agent,
        message: body.message || "",
        boundary: "mocked Capo server command",
        createdAt: new Date().toISOString()
      };
      commands.unshift(command);
      send(response, 200, JSON.stringify({ ok: true, command }));
      return;
    }
    if (request.method === "GET") {
      await serveStatic(url.pathname, response);
      return;
    }
    send(response, 405, "method not allowed", "text/plain; charset=utf-8");
  } catch (error) {
    send(response, 500, JSON.stringify({ error: error.message }));
  }
});

const port = Number.parseInt(process.env.CAPO_DASHBOARD_PORT || "4173", 10);
server.listen(port, "127.0.0.1", () => {
  console.log(`capo_dashboard_url=http://127.0.0.1:${port}`);
});
