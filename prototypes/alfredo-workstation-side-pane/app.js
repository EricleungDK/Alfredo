const VARIANTS = [
  { key: "cards", label: "A - Live cards" },
  { key: "table", label: "B - Dense table" },
];

const baseSessions = [
  {
    id: "CTRL-01",
    name: "Controller",
    model: "frontier-reviewer",
    role: "Planning and governance",
    status: "reviewing",
    task: "Review workstation side-pane interaction model before production work starts.",
    mission: "Desktop workstation shape",
    phase: "4/6",
    last: "32s ago",
    files: 2,
    approval: "Clear",
    next: "Compare action visibility across both variants.",
    tools: ["read CONTEXT.md", "read PRD #18", "summarize constraints"],
    command: "No shell command running",
    touched: ["CONTEXT.md", ".agent/issues/18-rebuild-mission-control-as-agent-workstation-prd.md"],
    evidence: ["Design-risk note", "Prompt dominance check"],
    action: "request review",
  },
  {
    id: "CODE-07",
    name: "Local coding agent",
    model: "qwen3.6-27b",
    role: "Implementation",
    status: "running",
    task: "Edit the workstation shell and wire mocked agent events into the UI.",
    mission: "Mission control UI",
    phase: "3/5",
    last: "12s ago",
    files: 7,
    approval: "Clear",
    next: "Finish side-pane event reducers, then run focused UI tests.",
    tools: ["rg workstation", "edit App.tsx", "npm test -- App"],
    command: "npm test -- App.test.tsx",
    touched: ["mission-control/src/App.tsx", "mission-control/src/App.test.tsx", "mission-control/src/styles.css"],
    evidence: ["Diff: workstation shell", "Test run in progress"],
    action: "cancel",
  },
  {
    id: "TEST-03",
    name: "Delegate subagent",
    model: "qwen2.5-coder-14b",
    role: "Verification",
    status: "thinking",
    task: "Minimize a failing side-pane selection test into a focused reproduction.",
    mission: "Regression repair",
    phase: "2/4",
    last: "1m ago",
    files: 1,
    approval: "Clear",
    next: "Decide whether stale selected-session state is reducer or fixture drift.",
    tools: ["inspect test fixture", "compare reducer branches", "draft hypothesis"],
    command: "No command running",
    touched: ["mission-control/src/App.test.tsx"],
    evidence: ["Failing assertion", "Reducer trace"],
    action: "open diff",
  },
  {
    id: "ADHOC-12",
    name: "Launch candidate",
    model: "qwen2.5-coder-14b",
    role: "Bounded task agent",
    status: "waiting approval",
    task: "Launch bounded session to rename public command examples from Albert to Alfredo.",
    mission: "Public rename staging",
    phase: "1/5",
    last: "3m ago",
    files: 4,
    approval: "Needs launch approval",
    next: "Approve launch after allowed paths are confirmed.",
    tools: ["prepare launch brief", "check allowed paths", "estimate blast radius"],
    command: "Pending user approval",
    touched: ["docs/agents/domain.md", ".scratch/albert-mission-control-app/PRD.md", "mission-control/package.json", "albert_mvp/cli.py"],
    evidence: ["Launch brief", "Allowed paths preview"],
    action: "approve launch",
  },
  {
    id: "FIX-09",
    name: "Repair session",
    model: "qwen3.6-27b",
    role: "Repair",
    status: "failed",
    task: "Repair package metadata smoke test after desktop package rename experiment.",
    mission: "NPM package readiness",
    phase: "5/5",
    last: "9m ago",
    files: 3,
    approval: "Needs review",
    next: "Retry from clean fixture or request controller review.",
    tools: ["npm test", "inspect package.json", "capture failure"],
    command: "npm test exited 1: expected package name albert-mission-control",
    touched: ["mission-control/package.json", "mission-control/package-lock.json", "mission-control/src/App.test.tsx"],
    evidence: ["Failure log", "Package diff"],
    action: "retry",
  },
];

let state = {
  variant: getVariant(),
  selectedId: "ADHOC-12",
  expandedIds: new Set(["ADHOC-12"]),
  sessions: structuredClone(baseSessions),
  promptTurns: [
    {
      type: "user",
      title: "Build the Alfredo workstation side-pane prototype.",
      body: "Compare live cards and dense table models while keeping prompt work dominant.",
    },
    {
      type: "assistant",
      title: "Controller response",
      body: "Loaded the workstation PRD, current app shape, and rename constraints. Side-pane actions will be routed through orchestrator-visible turns.",
    },
  ],
  stream: [],
  busyActionId: null,
  sideNotice: "Select, expand, filter, and diff inspection stay in the side pane. Approve, retry, cancel, and review requests become visible prompt turns.",
};

function getVariant() {
  const param = new URLSearchParams(window.location.search).get("variant");
  return VARIANTS.some((variant) => variant.key === param) ? param : "cards";
}

function setVariant(nextKey) {
  state.variant = nextKey;
  const params = new URLSearchParams(window.location.search);
  params.set("variant", nextKey);
  window.history.replaceState({}, "", `${window.location.pathname}?${params.toString()}`);
  render();
}

function selectedSession() {
  return state.sessions.find((session) => session.id === state.selectedId) || state.sessions[0];
}

function updateSession(id, patch) {
  state.sessions = state.sessions.map((session) => (session.id === id ? { ...session, ...patch } : session));
}

function statusText(status) {
  const labels = {
    thinking: "Thinking",
    running: "Running command",
    "waiting approval": "Waiting approval",
    blocked: "Blocked",
    reviewing: "Reviewing",
    done: "Done",
    failed: "Failed",
    launching: "Launching",
  };
  return labels[status] || status;
}

function statusClass(status) {
  return status.replace(/\s+/g, "-");
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function progressPercent(phase) {
  const [done, total] = phase.split("/").map(Number);
  if (!done || !total) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

function render() {
  document.querySelector("#app").innerHTML = `
    <section class="workstation-shell">
      <header class="top-bar">
        <div>
          <p class="eyebrow">Alfredo desktop workstation prototype</p>
          <h1>Prompt-first coding surface with persistent Agent Workstations</h1>
        </div>
        <div class="run-strip" aria-label="Run context">
          <span>CLI: <strong>alfredo</strong></span>
          <span>Agent: <strong>qwen3.6-27b</strong></span>
          <span>Mode: <strong>Desktop</strong></span>
        </div>
      </header>

      <div class="workspace-grid">
        <section class="prompt-pane" aria-label="Prompt pane">
          ${renderPromptPane()}
        </section>
        <aside class="agent-pane" aria-label="Persistent agent workstations">
          ${renderAgentPane()}
        </aside>
      </div>
    </section>
  `;
  renderSwitcher();
  bindEvents();
}

function renderPromptPane() {
  return `
    <div class="pane-header">
      <div>
        <p class="section-label">Primary working surface</p>
        <h2>Terminal prompt and turn scrollback</h2>
      </div>
      <button class="ghost-button" data-action="append-prompt">Add mock prompt</button>
    </div>

    <div class="timeline" aria-label="Visible transcript">
      ${state.promptTurns.map(renderTurn).join("")}
      ${state.stream.length ? renderStreamingTurn() : ""}
    </div>

    <div class="terminal-composer" aria-label="Prompt composer">
      <div class="terminal-status">
        <span>alfredo workstation</span>
        <span>cwd: local-coding-agent</span>
        <span>orchestrator governed</span>
      </div>
      <div class="prompt-input">
        <span class="prompt-caret">&gt;</span>
        <span>Plan the next bounded Alfredo workstation change...</span>
      </div>
    </div>
  `;
}

function renderTurn(turn) {
  return `
    <article class="turn ${turn.type}">
      <div class="turn-rail">${turn.type === "user" ? "U" : "O"}</div>
      <div>
        <h3>${escapeHtml(turn.title)}</h3>
        <p>${escapeHtml(turn.body)}</p>
      </div>
    </article>
  `;
}

function renderStreamingTurn() {
  return `
    <article class="turn orchestrator live">
      <div class="turn-rail">O</div>
      <div>
        <h3>Live orchestrator response</h3>
        <ol class="stream-list">
          ${state.stream.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}
        </ol>
      </div>
    </article>
  `;
}

function renderAgentPane() {
  return `
    <div class="agent-pane-header">
      <div>
        <p class="section-label">Persistent side pane</p>
        <h2>Agent Workstations</h2>
      </div>
      <div class="variant-tabs" role="tablist" aria-label="Prototype variants">
        ${VARIANTS.map(
          (variant) => `
            <button
              class="tab ${state.variant === variant.key ? "active" : ""}"
              role="tab"
              aria-selected="${state.variant === variant.key}"
              data-variant="${variant.key}"
            >${variant.key === "cards" ? "Cards" : "Table"}</button>
          `,
        ).join("")}
      </div>
    </div>

    <div class="agent-summary" aria-label="Agent summary">
      ${summaryMetric("Active", state.sessions.filter((session) => ["thinking", "running", "reviewing", "launching"].includes(session.status)).length)}
      ${summaryMetric("Approval", state.sessions.filter((session) => session.status === "waiting approval").length)}
      ${summaryMetric("Repair", state.sessions.filter((session) => session.status === "failed").length)}
    </div>
    <div class="side-notice">${escapeHtml(state.sideNotice)}</div>

    ${state.variant === "cards" ? renderCardsVariant() : renderTableVariant()}
  `;
}

function summaryMetric(label, value) {
  return `
    <div class="summary-metric">
      <strong>${value}</strong>
      <span>${label}</span>
    </div>
  `;
}

function renderCardsVariant() {
  return `
    <div class="variant-note">
      Compact live cards emphasize human-readable state and immediate action.
    </div>
    <div class="card-stack">
      ${state.sessions.map(renderSessionCard).join("")}
    </div>
  `;
}

function renderSessionCard(session) {
  const expanded = state.expandedIds.has(session.id);
  const selected = state.selectedId === session.id;
  return `
    <article class="session-card ${selected ? "selected" : ""}">
      <button class="card-hit-area" data-action="select" data-id="${session.id}" aria-label="Select ${session.id}"></button>
      <div class="card-topline">
        <div>
          <span class="session-id">${session.id}</span>
          <h3>${escapeHtml(session.name)}</h3>
        </div>
        <span class="status ${statusClass(session.status)}">${statusText(session.status)}</span>
      </div>
      <p class="task">${escapeHtml(session.task)}</p>
      <div class="meta-grid">
        <span>${escapeHtml(session.model)}</span>
        <span>${escapeHtml(session.role)}</span>
        <span>${escapeHtml(session.last)}</span>
        <span>${session.files} files</span>
      </div>
      <div class="progress-line" aria-label="Progress ${session.phase}">
        <span style="width: ${progressPercent(session.phase)}%"></span>
      </div>
      <div class="card-actions">
        ${renderPrimaryAction(session)}
        <button class="icon-button" data-action="toggle-expand" data-id="${session.id}" aria-label="${expanded ? "Collapse" : "Expand"} details">${expanded ? "-" : "+"}</button>
      </div>
      ${expanded ? renderDetails(session, "card") : ""}
    </article>
  `;
}

function renderTableVariant() {
  const selected = selectedSession();
  return `
    <div class="variant-note">
      Dense table optimizes scan speed, then moves full context into a stable inspector.
    </div>
    <div class="table-layout">
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Agent</th>
              <th>Status</th>
              <th>Task</th>
              <th>Mission</th>
              <th>Files</th>
              <th>Last</th>
              <th>Next</th>
            </tr>
          </thead>
          <tbody>
            ${state.sessions.map(renderSessionRow).join("")}
          </tbody>
        </table>
      </div>
      <section class="inspector" aria-label="Selected agent detail">
        <div class="inspector-title">
          <div>
            <span class="session-id">${selected.id}</span>
            <h3>${escapeHtml(selected.name)}</h3>
          </div>
          <span class="status ${statusClass(selected.status)}">${statusText(selected.status)}</span>
        </div>
        <p class="task">${escapeHtml(selected.task)}</p>
        <div class="inspector-actions">
          ${renderPrimaryAction(selected)}
          <button class="ghost-button" data-action="request-review" data-id="${selected.id}">Request review</button>
          <button class="ghost-button" data-action="open-diff" data-id="${selected.id}">Open diff</button>
        </div>
        ${renderDetails(selected, "inspector")}
      </section>
    </div>
  `;
}

function renderSessionRow(session) {
  return `
    <tr class="${state.selectedId === session.id ? "selected" : ""}" data-action="select" data-id="${session.id}">
      <td>
        <strong>${escapeHtml(session.id)}</strong>
        <span>${escapeHtml(session.name)} / ${escapeHtml(session.model)}</span>
      </td>
      <td><span class="status ${statusClass(session.status)}">${statusText(session.status)}</span></td>
      <td>${escapeHtml(session.task)}</td>
      <td>${escapeHtml(session.mission)}</td>
      <td>${session.files}</td>
      <td>${escapeHtml(session.last)}</td>
      <td>${escapeHtml(session.next)}</td>
    </tr>
  `;
}

function renderPrimaryAction(session) {
  if (session.status === "waiting approval") {
    return `<button class="action-button approve" data-action="approve-launch" data-id="${session.id}" ${state.busyActionId ? "disabled" : ""}>Approve launch</button>
      <button class="ghost-button" data-action="reject" data-id="${session.id}" ${state.busyActionId ? "disabled" : ""}>Reject</button>`;
  }
  if (session.status === "failed") {
    return `<button class="action-button retry" data-action="retry" data-id="${session.id}" ${state.busyActionId ? "disabled" : ""}>Retry</button>`;
  }
  if (session.status === "running" || session.status === "launching") {
    return `<button class="ghost-button" data-action="cancel" data-id="${session.id}" ${state.busyActionId ? "disabled" : ""}>Cancel</button>`;
  }
  if (session.action === "open diff") {
    return `<button class="ghost-button" data-action="open-diff" data-id="${session.id}">Open diff</button>`;
  }
  return `<button class="ghost-button" data-action="request-review" data-id="${session.id}">Request review</button>`;
}

function renderDetails(session, mode) {
  return `
    <div class="details ${mode}">
      <div>
        <h4>Recent tool activity</h4>
        <ul>${session.tools.map((tool) => `<li>${escapeHtml(tool)}</li>`).join("")}</ul>
      </div>
      <div>
        <h4>Command summary</h4>
        <p>${escapeHtml(session.command)}</p>
      </div>
      <div>
        <h4>Files touched</h4>
        <ul>${session.touched.map((file) => `<li>${escapeHtml(file)}</li>`).join("")}</ul>
      </div>
      <div>
        <h4>Evidence and next action</h4>
        <ul>${session.evidence.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>
        <p class="next-action">${escapeHtml(session.next)}</p>
      </div>
    </div>
  `;
}

function renderSwitcher() {
  const currentIndex = VARIANTS.findIndex((variant) => variant.key === state.variant);
  const current = VARIANTS[currentIndex];
  document.querySelector("#prototype-switcher").innerHTML = `
    <button class="switch-arrow" data-action="previous-variant" aria-label="Previous variant">&lt;</button>
    <span>${current.label}</span>
    <button class="switch-arrow" data-action="next-variant" aria-label="Next variant">&gt;</button>
  `;
}

function bindEvents() {
  document.querySelectorAll("[data-variant]").forEach((button) => {
    button.addEventListener("click", () => setVariant(button.dataset.variant));
  });

  document.querySelectorAll("[data-action]").forEach((element) => {
    element.addEventListener("click", (event) => {
      const action = element.dataset.action;
      const id = element.dataset.id;
      if (action === "select") selectSession(id);
      if (action === "toggle-expand") toggleExpand(id);
      if (action === "approve-launch") approveLaunch(id);
      if (action === "retry") retrySession(id);
      if (action === "reject") rejectLaunch(id);
      if (action === "cancel") appendUtilityTurn(id, "Cancel requested", "Visible controller turn would stop the bounded session after current tool call.");
      if (action === "request-review") appendUtilityTurn(id, "Review requested", "Controller records the request and schedules a frontier review turn.");
      if (action === "open-diff") sidePaneOnly(id, "Diff opened in the side pane without adding prompt transcript noise.");
      if (action === "append-prompt") appendMockPrompt();
      if (action === "previous-variant") cycleVariant(-1);
      if (action === "next-variant") cycleVariant(1);
      event.stopPropagation();
    });
  });
}

function selectSession(id) {
  state.selectedId = id;
  state.sideNotice = `${id} selected. Selection is local side-pane navigation.`;
  if (state.variant === "cards") state.expandedIds.add(id);
  render();
}

function toggleExpand(id) {
  if (state.expandedIds.has(id)) {
    state.expandedIds.delete(id);
  } else {
    state.expandedIds.add(id);
  }
  state.selectedId = id;
  render();
}

function cycleVariant(direction) {
  const currentIndex = VARIANTS.findIndex((variant) => variant.key === state.variant);
  const nextIndex = (currentIndex + direction + VARIANTS.length) % VARIANTS.length;
  setVariant(VARIANTS[nextIndex].key);
}

function appendMockPrompt() {
  state.promptTurns.push({
    type: "user",
    title: "User prompt",
    body: "Show me what changed and whether any agent is blocked.",
  });
  state.promptTurns.push({
    type: "assistant",
    title: "Controller response",
    body: "Two agents are active. ADHOC-12 still needs launch approval; FIX-09 needs retry or review.",
  });
  render();
}

function approveLaunch(id) {
  const session = state.sessions.find((candidate) => candidate.id === id);
  if (!session) return;
  state.busyActionId = id;
  state.selectedId = id;
  state.expandedIds.add(id);
  updateSession(id, {
    status: "launching",
    approval: "Approved",
    next: "Waiting for first bounded-session event.",
    command: "alfredo run --agent qwen2.5-coder-14b \"rename public examples to Alfredo\"",
  });
  state.promptTurns.push({
    type: "user",
    title: `Approve launch for ${id}`,
    body: `Approve launch for ${id} using ${session.model}.`,
  });
  runMockStream(id, [
    "Validating current revision and dirty-file boundaries.",
    "Checking allowed paths for public rename staging.",
    "Launching bounded session with orchestrator governance.",
    "Waiting for first agent event.",
  ], () => {
    updateSession(id, {
      status: "running",
      last: "now",
      phase: "2/5",
      next: "Watch first edit event and confirm scope remains bounded.",
      tools: ["validate revision", "check allowed paths", "launch bounded session", "receive first agent event"],
      evidence: ["Approval transcript turn", "Launch event", "Allowed paths check"],
    });
  });
}

function retrySession(id) {
  const session = state.sessions.find((candidate) => candidate.id === id);
  if (!session) return;
  state.busyActionId = id;
  state.selectedId = id;
  state.expandedIds.add(id);
  updateSession(id, {
    status: "launching",
    approval: "Retry approved",
    next: "Preparing clean fixture before retry.",
    command: "alfredo run --agent qwen3.6-27b \"repair package metadata smoke test\"",
  });
  state.promptTurns.push({
    type: "user",
    title: `Retry ${id}`,
    body: `Retry failed repair session ${id} using ${session.model}.`,
  });
  runMockStream(id, [
    "Capturing previous failure as evidence.",
    "Preparing clean fixture and allowed paths.",
    "Starting retry with repair constraints.",
    "Waiting for focused test command.",
  ], () => {
    updateSession(id, {
      status: "running",
      last: "now",
      phase: "1/4",
      next: "Run focused metadata smoke test and report evidence.",
      tools: ["capture previous failure", "prepare clean fixture", "start retry"],
      evidence: ["Retry transcript turn", "Previous failure retained"],
    });
  });
}

function rejectLaunch(id) {
  updateSession(id, {
    status: "blocked",
    approval: "Rejected",
    next: "Revise launch brief before asking again.",
  });
  state.promptTurns.push({
    type: "user",
    title: `Reject launch for ${id}`,
    body: "Rejected from the side pane. The controller should revise scope before another launch request.",
  });
  render();
}

function appendUtilityTurn(id, title, body) {
  state.selectedId = id;
  state.promptTurns.push({
    type: "assistant",
    title: `${title}: ${id}`,
    body,
  });
  render();
}

function sidePaneOnly(id, message) {
  state.selectedId = id;
  state.expandedIds.add(id);
  state.sideNotice = `${id}: ${message}`;
  render();
}

function runMockStream(id, steps, onDone) {
  state.stream = [];
  render();
  steps.forEach((step, index) => {
    window.setTimeout(() => {
      state.stream = [...state.stream, step];
      render();
      if (index === steps.length - 1) {
        window.setTimeout(() => {
          state.promptTurns.push({
            type: "assistant",
            title: `Orchestrator accepted ${id}`,
            body: `${id} is now running. The side pane status changed from waiting approval to running, and this transcript records why.`,
          });
          state.stream = [];
          state.busyActionId = null;
          onDone();
          render();
        }, 650);
      }
    }, 550 * (index + 1));
  });
}

window.addEventListener("keydown", (event) => {
  const tag = document.activeElement?.tagName;
  const isEditable = document.activeElement?.isContentEditable || tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
  if (isEditable) return;
  if (event.key === "ArrowLeft") cycleVariant(-1);
  if (event.key === "ArrowRight") cycleVariant(1);
});

window.addEventListener("popstate", () => {
  state.variant = getVariant();
  render();
});

render();
