import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(fileURLToPath(new URL("..", import.meta.url)));
const [html, css, js, fixtureText] = await Promise.all([
  readFile(join(root, "index.html"), "utf8"),
  readFile(join(root, "styles.css"), "utf8"),
  readFile(join(root, "app.js"), "utf8"),
  readFile(join(root, "fixtures", "dashboard.json"), "utf8")
]);
const fixture = JSON.parse(fixtureText);

const checks = [
  ["app shell exists", html.includes('data-testid="dashboard-app"')],
  ["status metrics exist", html.includes("metric-agents") && html.includes("metric-validations")],
  ["agent list exists", html.includes("agent-list")],
  ["command panel exists", html.includes("steer-button") && html.includes("interrupt-button")],
  ["debug drawer exists", html.includes("debug-drawer")],
  ["responsive CSS exists", css.includes("@media (max-width: 980px)") && css.includes("@media (max-width: 520px)")],
  ["no viewport font scaling", !/font-size\s*:[^;]*vw/.test(css)],
  ["fixture has agents", Array.isArray(fixture.agents) && fixture.agents.length >= 3],
  ["fixture has evidence", Array.isArray(fixture.evidence) && fixture.evidence.length > 0],
  ["fixture has goals", Array.isArray(fixture.goals) && fixture.goals.length > 0],
  ["mock command API used", js.includes('fetch("/api/commands"')],
  ["results remain hidden behind details", html.includes("Toggle details") && fixture.project.mode === "fixture"]
];

const failed = checks.filter(([, ok]) => !ok);
for (const [name, ok] of checks) {
  console.log(`${ok ? "ok" : "FAIL"} ${name}`);
}
if (failed.length > 0) {
  process.exitCode = 1;
}
