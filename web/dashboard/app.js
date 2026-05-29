const state = {
  data: null,
  selectedAgent: null,
  commandLog: [],
  view: "overview"
};

const el = (id) => document.getElementById(id);

function statusClass(status) {
  return status.toLowerCase().replace(/\s+/g, "-");
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

async function loadDashboard() {
  const response = await fetch("/api/dashboard");
  if (!response.ok) {
    throw new Error(`dashboard request failed: ${response.status}`);
  }
  state.data = await response.json();
  state.selectedAgent ||= state.data.agents[0]?.name || null;
  render();
}

function render() {
  renderShell();
  renderMetrics();
  renderAgents();
  renderDetail();
  renderActivity();
  renderLanes();
  renderGoals();
  renderSettings();
  renderDebug();
}

function renderShell() {
  el("server-badge").textContent = state.data.project.server;
  document.body.classList.toggle("settings-active", state.view === "settings");
  document.querySelectorAll(".tab").forEach((button) => {
    button.classList.toggle("is-active", button.dataset.view === state.view);
  });
  const showGoals = state.view === "goals";
  const showSettings = state.view === "settings";
  el("goals-view").classList.toggle("hidden", !showGoals);
  el("settings-view").classList.toggle("hidden", !showSettings);
}

function renderMetrics() {
  const summary = state.data.summary;
  el("metric-agents").textContent = summary.agents;
  el("metric-active").textContent = summary.active;
  el("metric-blocked").textContent = summary.blocked;
  el("metric-evidence").textContent = summary.evidence;
  el("metric-reviews").textContent = summary.reviews;
  el("metric-validations").textContent = summary.validations;
}

function renderAgents() {
  const filter = el("agent-filter").value.trim().toLowerCase();
  const agents = state.data.agents.filter((agent) => {
    const haystack = `${agent.name} ${agent.status} ${agent.adapter}`.toLowerCase();
    return !filter || haystack.includes(filter);
  });
  el("agent-list").innerHTML = agents
    .map((agent) => `
      <button class="agent-row ${agent.name === state.selectedAgent ? "is-selected" : ""}" type="button" data-agent="${escapeHtml(agent.name)}" role="listitem">
        <span>
          <span class="agent-name">${escapeHtml(agent.name)}</span>
          <span class="agent-meta">${escapeHtml(agent.adapter)} · ${agent.tools} tools · ${agent.memory} memories</span>
        </span>
        <span class="status ${statusClass(agent.status)}">${escapeHtml(agent.status)}</span>
        <span class="small-muted">${escapeHtml(agent.result)}</span>
        <span class="small-muted">${agent.evidence.length} evidence · ${agent.reviews} reviews</span>
      </button>
    `)
    .join("") || `<p class="small-muted">No agents match the filter.</p>`;
}

function selectedAgent() {
  return state.data.agents.find((agent) => agent.name === state.selectedAgent) || state.data.agents[0];
}

function renderDetail() {
  const agent = selectedAgent();
  if (!agent) {
    el("agent-detail").innerHTML = `<p>No agents yet.</p>`;
    el("selected-status").textContent = "empty";
    return;
  }
  el("selected-status").textContent = agent.status;
  el("selected-status").className = `badge ${statusClass(agent.status)}`;
  el("agent-detail").innerHTML = `
    <div class="detail-stack">
      <div class="result-block">
        <h3>Latest Result</h3>
        <p>${escapeHtml(agent.result)}</p>
      </div>
      <div class="metadata-grid">
        <div><span>Goal</span><strong>${escapeHtml(agent.goal)}</strong></div>
        <div><span>Confidence</span><strong>${escapeHtml(agent.confidence)}</strong></div>
        <div><span>Evidence</span><strong>${agent.evidence.map(escapeHtml).join(", ") || "none"}</strong></div>
        <div><span>Reviews</span><strong>${agent.reviews}</strong></div>
        <div><span>Validations</span><strong>${agent.validations}</strong></div>
        <div><span>Blocker</span><strong>${escapeHtml(agent.blocker || "none")}</strong></div>
      </div>
    </div>
  `;
}

function renderActivity() {
  el("activity-list").innerHTML = state.data.activity
    .map((item) => `
      <li>
        <strong>${escapeHtml(item.time)} ${escapeHtml(item.agent)}</strong>
        <span class="small-muted">${escapeHtml(item.kind)}</span>
        <div>${escapeHtml(item.text)}</div>
      </li>
    `)
    .join("");
}

function renderLanes() {
  renderCompactList("evidence-list", state.data.evidence, (item) => `${item.id} · ${item.kind} · ${item.status}`);
  renderCompactList("review-list", state.data.reviews, (item) => `${item.id} · ${item.target} · ${item.status}`);
  renderCompactList("validation-list", state.data.validations, (item) => `${item.id} · ${item.target} · ${item.status}`);
}

function renderCompactList(id, rows, format) {
  el(id).innerHTML = rows
    .map((row) => `<div class="compact-row">${escapeHtml(format(row))}</div>`)
    .join("") || `<div class="compact-row">none</div>`;
}

function renderGoals() {
  el("goal-list").innerHTML = state.data.goals
    .map((goal) => `
      <article class="goal-row">
        <strong>${escapeHtml(goal.title)}</strong>
        <span class="status ${statusClass(goal.status)}">${escapeHtml(goal.status)}</span>
        <ul>
          ${goal.requirements.map((req) => `<li>${escapeHtml(req)}</li>`).join("")}
        </ul>
        <p class="small-muted">Validation: ${escapeHtml(goal.validation)}</p>
        <p class="small-muted">Blockers: ${goal.blockers.map(escapeHtml).join(", ") || "none"}</p>
      </article>
    `)
    .join("");
}

function renderSettings() {
  el("settings-mode").textContent = state.data.project.mode;
}

function renderDebug() {
  const agent = selectedAgent();
  const payload = {
    project: state.data.project,
    selectedAgent: agent?.debug || null,
    commandLog: state.commandLog
  };
  el("debug-output").textContent = JSON.stringify(payload, null, 2);
}

async function submitCommand(kind) {
  const agent = selectedAgent();
  if (!agent) {
    return;
  }
  const message = kind === "steer_agent" ? el("steer-input").value.trim() : kind.replace("_agent", "");
  if (kind === "steer_agent" && !message) {
    el("command-log").textContent = "Write a steering message before sending.";
    return;
  }
  const response = await fetch("/api/commands", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ kind, agent: agent.name, message })
  });
  const result = await response.json();
  state.commandLog.unshift(result);
  el("command-log").textContent = `${result.command.kind} queued for ${result.command.agent}`;
  if (kind === "steer_agent") {
    el("steer-input").value = "";
  }
  renderDebug();
}

function bindEvents() {
  el("refresh-button").addEventListener("click", loadDashboard);
  el("debug-toggle").addEventListener("click", () => {
    el("debug-drawer").hidden = !el("debug-drawer").hidden;
  });
  el("debug-close").addEventListener("click", () => {
    el("debug-drawer").hidden = true;
  });
  el("agent-filter").addEventListener("input", renderAgents);
  el("agent-list").addEventListener("click", (event) => {
    const row = event.target.closest("[data-agent]");
    if (!row) {
      return;
    }
    state.selectedAgent = row.dataset.agent;
    render();
  });
  document.querySelectorAll(".tab").forEach((button) => {
    button.addEventListener("click", () => {
      state.view = button.dataset.view;
      render();
    });
  });
  el("steer-button").addEventListener("click", () => submitCommand("steer_agent"));
  el("interrupt-button").addEventListener("click", () => submitCommand("interrupt_agent"));
  el("stop-button").addEventListener("click", () => submitCommand("stop_agent"));
}

bindEvents();
loadDashboard().catch((error) => {
  el("server-badge").textContent = "offline";
  el("agent-detail").innerHTML = `<p class="small-muted">${escapeHtml(error.message)}</p>`;
});

if (new URLSearchParams(window.location.search).get("qa") === "mobile") {
  el("app").classList.add("qa-mobile");
}
