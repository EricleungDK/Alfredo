const VARIANTS = [
  { key: "report", label: "A - Generated review report" },
  { key: "mission", label: "B - Mission-control board" },
  { key: "hybrid", label: "C - Hybrid review workspace" },
];

const agents = [
  "qwen-coder-local-1",
  "qwen-coder-local-2",
  "frontier-architect",
  "frontier-reviewer",
  "hold-for-user",
];

const baseIssues = [
  {
    id: "ISS-01",
    title: "Create mission shell and shared context store",
    summary: "Build the first navigable shell and a read-only Shared Context surface for coding missions.",
    mode: "HITL",
    risk: "High",
    status: "pending",
    suggestedAgent: "frontier-architect",
    assignedAgent: "frontier-architect",
    blockedBy: [],
    stories: ["As a user, I can see the mission state before agents start work."],
    acceptance: [
      "Mission shell shows goal, constraints, and accepted decisions.",
      "Shared Context has a factual log and proposal queue.",
      "No local agent can directly mutate Shared Context.",
    ],
    evidence: ["Architecture sketch", "Boundary list", "Manual walkthrough"],
  },
  {
    id: "ISS-02",
    title: "Generate Issue Slices from an approved PRD",
    summary: "Transform a PRD into vertical implementation slices with blockers, labels, and verification hooks.",
    mode: "HITL",
    risk: "High",
    status: "pending",
    suggestedAgent: "frontier-architect",
    assignedAgent: "frontier-architect",
    blockedBy: ["ISS-01"],
    stories: ["As a user, I approve the slice plan before any model-agent assignment."],
    acceptance: [
      "Each Issue Slice is independently grabbable and verifiable.",
      "Slices include blocker relationships and HITL or AFK label.",
      "The user can request revise, split, merge, or hold before execution.",
    ],
    evidence: ["Generated issue graph", "Slice rationale", "Open questions"],
  },
  {
    id: "ISS-03",
    title: "Assign local model agents with user override",
    summary: "Recommend local worker assignments while preserving explicit user control before launch.",
    mode: "AFK",
    risk: "Medium",
    status: "pending",
    suggestedAgent: "qwen-coder-local-1",
    assignedAgent: "qwen-coder-local-1",
    blockedBy: ["ISS-02"],
    stories: ["As a user, I can accept suggested assignments or override them one slice at a time."],
    acceptance: [
      "Default assignment is visible before execution.",
      "User override is reflected in the launch summary.",
      "Frontier roles remain planning and review roles, not default editors.",
    ],
    evidence: ["Assignment matrix", "Cost estimate", "Capability rationale"],
  },
  {
    id: "ISS-04",
    title: "Collect evidence packages from local agents",
    summary: "Define the completion package required before frontier review accepts or rejects a slice.",
    mode: "AFK",
    risk: "Medium",
    status: "pending",
    suggestedAgent: "qwen-coder-local-2",
    assignedAgent: "qwen-coder-local-2",
    blockedBy: ["ISS-03"],
    stories: ["As a reviewer, I see test output, changed files, risks, and proposed context updates together."],
    acceptance: [
      "Evidence Package includes changed files, diff, tests, risks, and context proposals.",
      "Missing evidence blocks frontier approval.",
      "Rejected work follows the tiered repair policy.",
    ],
    evidence: ["Package schema", "Validation errors", "Repair decision log"],
  },
  {
    id: "ISS-05",
    title: "Transition approved slices to execution board",
    summary: "Move approved Issue Slices into an execution view with dependency-aware launch readiness.",
    mode: "HITL",
    risk: "Low",
    status: "pending",
    suggestedAgent: "frontier-integrator",
    assignedAgent: "frontier-integrator",
    blockedBy: ["ISS-02", "ISS-03"],
    stories: ["As a user, I can see exactly what will launch and what remains blocked."],
    acceptance: [
      "Approved unblocked slices are marked launchable.",
      "Blocked slices show the blocker and next required decision.",
      "Execution board keeps review decisions visible.",
    ],
    evidence: ["Launch preview", "Blocked-by grouping", "Integrator checklist"],
  },
];

let state = {
  selectedId: "ISS-02",
  activity: ["Prototype loaded with mock PRD-derived Issue Slices."],
  issues: structuredClone(baseIssues),
  stage: "review",
};

function getVariant() {
  const param = new URLSearchParams(window.location.search).get("variant");
  return VARIANTS.some((variant) => variant.key === param) ? param : "report";
}

function setVariant(nextKey) {
  const params = new URLSearchParams(window.location.search);
  params.set("variant", nextKey);
  window.history.replaceState({}, "", `${window.location.pathname}?${params.toString()}`);
  render();
}

function selectedIssue() {
  return state.issues.find((issue) => issue.id === state.selectedId) || state.issues[0];
}

function updateIssue(id, patch, message) {
  state.issues = state.issues.map((issue) => (issue.id === id ? { ...issue, ...patch } : issue));
  state.selectedId = id;
  state.activity = [`${id}: ${message}`, ...state.activity].slice(0, 8);
  render();
}

function selectIssue(id) {
  state.selectedId = id;
  state.activity = [`${id}: selected for inspection.`, ...state.activity].slice(0, 8);
  render();
}

function overrideAgent(id, agent) {
  updateIssue(id, { assignedAgent: agent }, `assignment overridden to ${agent}.`);
}

function statusLabel(status) {
  const labels = {
    pending: "Needs decision",
    approved: "Approved",
    revise: "Revise",
    split: "Split requested",
    merge: "Merge requested",
    held: "Held",
  };
  return labels[status] || status;
}

function riskClass(risk) {
  return `risk-${risk.toLowerCase()}`;
}

function canLaunch(issue) {
  return (
    issue.status === "approved" &&
    issue.blockedBy.every((blocker) => {
      const blockerIssue = state.issues.find((candidate) => candidate.id === blocker);
      return blockerIssue && blockerIssue.status === "approved";
    })
  );
}

function dependencyDepth(issue, seen = new Set()) {
  if (seen.has(issue.id) || issue.blockedBy.length === 0) return 0;
  seen.add(issue.id);
  return (
    1 +
    Math.max(
      ...issue.blockedBy.map((blockerId) => {
        const blocker = state.issues.find((candidate) => candidate.id === blockerId);
        return blocker ? dependencyDepth(blocker, new Set(seen)) : 0;
      }),
    )
  );
}

function openBlockers(issue) {
  return issue.blockedBy.filter((blockerId) => {
    const blocker = state.issues.find((candidate) => candidate.id === blockerId);
    return !blocker || blocker.status !== "approved";
  });
}

function orderedIssues() {
  const originalIndex = new Map(state.issues.map((issue, index) => [issue.id, index]));
  return [...state.issues].sort((a, b) => {
    const aDepth = dependencyDepth(a);
    const bDepth = dependencyDepth(b);
    if (aDepth !== bDepth) return aDepth - bDepth;
    return originalIndex.get(a.id) - originalIndex.get(b.id);
  });
}

function issueSequence(issue) {
  return orderedIssues().findIndex((candidate) => candidate.id === issue.id) + 1;
}

function metrics() {
  const approved = state.issues.filter((issue) => issue.status === "approved").length;
  const blocked = state.issues.filter((issue) => issue.blockedBy.length > 0 && !canLaunch(issue)).length;
  const overrides = state.issues.filter((issue) => issue.assignedAgent !== issue.suggestedAgent).length;
  return { approved, blocked, overrides };
}

function issueActions(issue) {
  return `
    <div class="compact-actions" aria-label="Actions for ${issue.id}">
      <button class="action" data-action="approve" data-id="${issue.id}">Approve</button>
      <button class="ghost-action" data-action="revise" data-id="${issue.id}">Revise</button>
      <button class="ghost-action" data-action="split" data-id="${issue.id}">Split</button>
      <button class="ghost-action" data-action="merge" data-id="${issue.id}">Merge</button>
      <button class="danger-action" data-action="hold" data-id="${issue.id}">Hold</button>
    </div>
  `;
}

function issueCard(issue, compact = false) {
  const blockers = openBlockers(issue);
  const depth = dependencyDepth(issue);
  return `
    <article class="issue-card ${issue.id === state.selectedId ? "selected" : ""}" data-select="${issue.id}">
      <div class="issue-meta">
        <span class="sequence-chip">#${issueSequence(issue)}</span>
        <span class="mini-id">${issue.id}</span>
        <span class="tag">Gate ${depth}</span>
        <span class="pill ${issue.mode.toLowerCase()}">${issue.mode}</span>
        <span class="pill ${riskClass(issue.risk)}">${issue.risk} risk</span>
        <span class="status-chip">${statusLabel(issue.status)}</span>
      </div>
      <h3>${issue.title}</h3>
      <p class="issue-summary">${issue.summary}</p>
      <div class="issue-tags">
        <span class="tag ${blockers.length ? "blocked-tag" : "ready-tag"}">${blockers.length ? `Open blockers: ${blockers.join(", ")}` : "No open blockers"}</span>
        <span class="tag">Suggested: ${issue.suggestedAgent}</span>
        <span class="tag">Assigned: ${issue.assignedAgent}</span>
      </div>
      ${compact ? "" : issueActions(issue)}
    </article>
  `;
}

function assignmentControl(issue) {
  return `
    <label class="small muted" for="agent-${issue.id}">Override model assignment</label>
    <select class="assignment-select" id="agent-${issue.id}" data-agent-for="${issue.id}">
      ${agents
        .map((agent) => `<option value="${agent}" ${agent === issue.assignedAgent ? "selected" : ""}>${agent}</option>`)
        .join("")}
    </select>
  `;
}

function dependencyMap() {
  const ordered = orderedIssues();
  return `
    <div class="dependency-map">
      ${ordered
        .map((issue) => {
          const blockers = openBlockers(issue);
          const blocked = blockers.length > 0;
          const detail = issue.blockedBy.length
            ? `Depends on ${issue.blockedBy.join(", ")}${blockers.length ? ` - open: ${blockers.join(", ")}` : ""}`
            : "Root slice";
          return `
            <button class="dependency-node ${blocked ? "blocked" : ""}" data-select="${issue.id}">
              <span class="mini-id">#${issueSequence(issue)} ${issue.id}</span>
              <span>
                <strong>${issue.title}</strong><br />
                <span class="small muted">${detail}</span>
              </span>
            </button>
          `;
        })
        .join("")}
    </div>
  `;
}

function detailPanel(issue) {
  return `
    <section class="panel">
      <div class="panel-header">
        <div>
          <p class="eyebrow">Selected Issue Slice</p>
          <h2 class="panel-title">${issue.id} - ${issue.title}</h2>
        </div>
        <span class="status-chip">${statusLabel(issue.status)}</span>
      </div>
      <div class="panel-body detail-layout">
        <div class="detail-section">
          <h3>User stories covered</h3>
          <ul>${issue.stories.map((story) => `<li>${story}</li>`).join("")}</ul>
        </div>
        <div class="detail-section">
          <h3>Assignment</h3>
          <p class="muted small">Recommendation: ${issue.suggestedAgent}</p>
          ${assignmentControl(issue)}
        </div>
        <div class="detail-section">
          <h3>Acceptance criteria</h3>
          <ul>${issue.acceptance.map((item) => `<li>${item}</li>`).join("")}</ul>
        </div>
        <div class="detail-section">
          <h3>Evidence required</h3>
          <ul>${issue.evidence.map((item) => `<li>${item}</li>`).join("")}</ul>
        </div>
      </div>
    </section>
  `;
}

function stateDock() {
  const visible = orderedIssues().map((issue) => ({
    id: issue.id,
    sequence: issueSequence(issue),
    dependencyDepth: dependencyDepth(issue),
    status: issue.status,
    assignedAgent: issue.assignedAgent,
    blockedBy: issue.blockedBy,
    openBlockers: openBlockers(issue),
  }));
  return `
    <section class="panel state-dock">
      <div class="panel-header">
        <h2 class="panel-title">Visible prototype state</h2>
      </div>
      <div class="panel-body">
        <pre>${JSON.stringify({ selectedId: state.selectedId, stage: state.stage, issues: visible, activity: state.activity }, null, 2)}</pre>
      </div>
    </section>
  `;
}

function header(title, subtitle) {
  const currentMetrics = metrics();
  return `
    <header class="top-strip">
      <div>
        <p class="eyebrow">Throwaway prototype - Issue Review Board</p>
        <h1>${title}</h1>
        <p class="subtitle">${subtitle}</p>
      </div>
      <div class="mission-metrics" aria-label="Mission metrics">
        <div class="metric"><strong>${currentMetrics.approved}</strong><span>Approved</span></div>
        <div class="metric"><strong>${currentMetrics.blocked}</strong><span>Blocked</span></div>
        <div class="metric"><strong>${currentMetrics.overrides}</strong><span>Overrides</span></div>
      </div>
    </header>
  `;
}

function renderReport() {
  document.body.className = "report-page";
  return `
    <div class="app-shell">
      <div class="report-document">
        <section class="report-main">
          <p class="eyebrow">Variant A - generated HTML artifact</p>
          <h1>Issue Slice Review Packet</h1>
          <p class="subtitle">This version treats review as a generated, shareable document. It is strong for async reading and weak for rapid steering because every meaningful edit feels like annotating a report.</p>
          <div class="report-callout">
            <strong>Question this variant answers:</strong> is a generated review page enough if it includes approvals, assignment overrides, blocker grouping, and the execution preview?
          </div>
          <table class="report-table">
            <thead>
              <tr>
                <th>Slice</th>
                <th>Decision</th>
                <th>Blockers</th>
                <th>Assignment</th>
                <th>Controls</th>
              </tr>
            </thead>
            <tbody>
              ${orderedIssues()
                .map(
                  (issue) => `
                    <tr data-select="${issue.id}">
                      <td><strong>#${issueSequence(issue)} ${issue.id}</strong><br />${issue.title}<br /><span class="muted small">${issue.summary}</span></td>
                      <td><span class="pill ${issue.mode.toLowerCase()}">${issue.mode}</span> <span class="pill ${riskClass(issue.risk)}">${issue.risk}</span><br /><span class="small">${statusLabel(issue.status)}</span></td>
                      <td>${issue.blockedBy.length ? issue.blockedBy.join(", ") : "None"}<br /><span class="small muted">${openBlockers(issue).length ? `Open: ${openBlockers(issue).join(", ")}` : "Ready in sequence"}</span></td>
                      <td>${assignmentControl(issue)}</td>
                      <td>${issueActions(issue)}</td>
                    </tr>
                  `,
                )
                .join("")}
            </tbody>
          </table>
          <div class="report-callout">
            <strong>Execution preview:</strong> ${state.issues.filter(canLaunch).length} slices can launch now. ${state.issues.filter((issue) => issue.status !== "approved").length} still need review decisions.
          </div>
        </section>
        <aside class="report-aside">
          <h2>Blocker grouping</h2>
          ${dependencyMap()}
          <hr />
          <h2>Recent actions</h2>
          <ul class="timeline-list">${state.activity.map((item) => `<li>${item}</li>`).join("")}</ul>
        </aside>
      </div>
      ${stateDock()}
    </div>
  `;
}

function executionCard(issue) {
  const blockers = openBlockers(issue);
  return `
    <div class="execution-card" data-select="${issue.id}">
      <span class="signal ${canLaunch(issue) ? "approved" : issue.status}"></span>
      <span>
        <strong>#${issueSequence(issue)} ${issue.id}</strong><br />
        <span class="small muted">${canLaunch(issue) ? "Ready to launch" : blockers.length ? `Waiting on ${blockers.join(", ")}` : statusLabel(issue.status)}</span>
      </span>
      <span class="tag">${issue.assignedAgent}</span>
    </div>
  `;
}

function missionExecutionPanel() {
  const ordered = orderedIssues();
  const ready = ordered.filter(canLaunch);
  const blocked = ordered.filter((issue) => issue.status === "approved" && !canLaunch(issue));
  const decisions = ordered.filter((issue) => issue.status !== "approved");
  return `
    <section class="panel mission-execution-panel">
      <div class="panel-header">
        <div>
          <p class="eyebrow">Execution Board</p>
          <h2 class="panel-title">Dependency-aware launch plan</h2>
        </div>
        <button class="ghost-action" data-action="review">Back to review queue</button>
      </div>
      <div class="panel-body mission-execution-groups">
        <div class="execution-group">
          <h3>Ready to launch</h3>
          ${ready.map(executionCard).join("") || `<p class="muted">No approved unblocked slices yet.</p>`}
        </div>
        <div class="execution-group">
          <h3>Approved but blocked</h3>
          ${blocked.map(executionCard).join("") || `<p class="muted">No approved slices are currently blocked.</p>`}
        </div>
        <div class="execution-group">
          <h3>Needs review decision</h3>
          ${decisions.map(executionCard).join("") || `<p class="muted">All slices have review decisions.</p>`}
        </div>
      </div>
    </section>
  `;
}

function missionReviewPanel() {
  return `
    <section class="panel">
      <div class="panel-header">
        <h2 class="panel-title">Review Queue - dependency order</h2>
        <button class="action" data-action="execution">View execution board</button>
      </div>
      <div class="panel-body issue-list">
        ${orderedIssues().map((candidate) => issueCard(candidate)).join("")}
      </div>
    </section>
  `;
}

function renderMission() {
  document.body.className = "";
  const issue = selectedIssue();
  const inExecutionMode = state.stage === "execute";
  return `
    <div class="app-shell">
      ${header(
        inExecutionMode ? "Mission control execution board" : "Mission control for approving agent work packets",
        inExecutionMode
          ? "Variant B stays app-native: after review, the same board pivots into dependency-aware execution without routing to the hybrid concept."
          : "This version makes the Issue Review Board a permanent operational surface. The board emphasizes triage, blockers, and launch readiness over document reading.",
      )}
      <section class="board-grid">
        <aside class="panel">
          <div class="panel-header"><h2 class="panel-title">Issue Graph</h2></div>
          <div class="panel-body">${dependencyMap()}</div>
        </aside>
        ${inExecutionMode ? missionExecutionPanel() : missionReviewPanel()}
        <aside class="mission-detail-column">
          ${detailPanel(issue)}
        </aside>
        <aside>
          <section class="panel">
            <div class="panel-header"><h2 class="panel-title">Launch Readiness</h2></div>
            <div class="panel-body execution-lane">
              ${orderedIssues()
                .map(
                  (candidate) => `
                    <div class="execution-card">
                      <span class="signal ${canLaunch(candidate) ? "approved" : candidate.status}"></span>
                      <span><strong>#${issueSequence(candidate)} ${candidate.id}</strong><br /><span class="small muted">${canLaunch(candidate) ? "Ready to launch" : statusLabel(candidate.status)}</span></span>
                      <span class="tag">${candidate.assignedAgent}</span>
                    </div>
                  `,
                )
                .join("")}
            </div>
          </section>
          <br />
          ${stateDock()}
        </aside>
      </section>
    </div>
  `;
}

function renderHybrid() {
  document.body.className = "";
  const stages = ["review", "assign", "execute"];
  const ordered = orderedIssues();
  const byStage = {
    review: ordered.filter((issue) => ["pending", "revise", "split", "merge", "held"].includes(issue.status)),
    assign: ordered.filter((issue) => issue.status === "approved" && !canLaunch(issue)),
    execute: ordered.filter(canLaunch),
  };
  return `
    <div class="app-shell">
      <section class="hybrid-shell">
        <aside class="hybrid-nav">
          <p class="eyebrow">Variant C - hybrid</p>
          <h1 style="font-size: 34px; line-height: 1.05;">Review in app, export the artifact.</h1>
          <p class="subtitle">This version keeps decisions native to the mission-control UI, then generates a concise HTML summary for sharing or archival.</p>
          <br />
          <div class="detail-section">
            <h3>Export summary</h3>
            <p class="muted small">A generated report is available after approval, but the editable source of truth stays in the board.</p>
            <button class="ghost-action" data-action="export">Preview export packet</button>
          </div>
        </aside>
        <div class="hybrid-stage">
          <nav class="stage-tabs" aria-label="Hybrid stage tabs">
            ${stages.map((stage) => `<button class="stage-tab ${state.stage === stage ? "active" : ""}" data-stage="${stage}">${stage}</button>`).join("")}
          </nav>
          <div class="hybrid-content">
            <section class="kanban-grid">
              <div class="kanban-column">
                <h3>Review decisions</h3>
                ${byStage.review.map((issue) => issueCard(issue, false)).join("") || `<p class="muted">No slices waiting for review.</p>`}
              </div>
              <div class="kanban-column">
                <h3>Approved but blocked</h3>
                ${byStage.assign.map((issue) => issueCard(issue, true)).join("") || `<p class="muted">No approved slices are blocked.</p>`}
              </div>
              <div class="kanban-column">
                <h3>Execution ready</h3>
                ${byStage.execute.map((issue) => issueCard(issue, true)).join("") || `<p class="muted">Approve unblocked slices to populate this lane.</p>`}
              </div>
            </section>
            <aside>
              ${detailPanel(selectedIssue())}
              <br />
              ${stateDock()}
            </aside>
          </div>
        </div>
      </section>
    </div>
  `;
}

function renderSwitcher() {
  const current = getVariant();
  const index = VARIANTS.findIndex((variant) => variant.key === current);
  const previous = VARIANTS[(index - 1 + VARIANTS.length) % VARIANTS.length];
  const next = VARIANTS[(index + 1) % VARIANTS.length];
  document.getElementById("prototype-switcher").innerHTML = `
    <button class="switcher-button" data-variant="${previous.key}" aria-label="Previous variant">‹</button>
    <div class="switcher-label">${VARIANTS[index].label}</div>
    <button class="switcher-button" data-variant="${next.key}" aria-label="Next variant">›</button>
  `;
}

function render() {
  const variant = getVariant();
  const app = document.getElementById("app");
  if (variant === "mission") app.innerHTML = renderMission();
  else if (variant === "hybrid") app.innerHTML = renderHybrid();
  else app.innerHTML = renderReport();
  renderSwitcher();
}

document.addEventListener("click", (event) => {
  const variantTarget = event.target.closest("[data-variant]");
  if (variantTarget) {
    setVariant(variantTarget.dataset.variant);
    return;
  }

  const stageTarget = event.target.closest("[data-stage]");
  if (stageTarget) {
    state.stage = stageTarget.dataset.stage;
    state.activity = [`Stage switched to ${state.stage}.`, ...state.activity].slice(0, 8);
    render();
    return;
  }

  const selectTarget = event.target.closest("[data-select]");
  if (selectTarget && !event.target.closest("[data-action], select, [data-variant], [data-stage]")) {
    selectIssue(selectTarget.dataset.select);
    return;
  }

  const actionTarget = event.target.closest("[data-action]");
  if (!actionTarget) return;
  const id = actionTarget.dataset.id || state.selectedId;
  const action = actionTarget.dataset.action;
  if (action === "approve") updateIssue(id, { status: "approved" }, "approved for model-agent assignment.");
  if (action === "revise") updateIssue(id, { status: "revise" }, "sent back for frontier task revision.");
  if (action === "split") updateIssue(id, { status: "split" }, "marked for splitting into smaller Issue Slices.");
  if (action === "merge") updateIssue(id, { status: "merge" }, "marked for merge review with neighboring slices.");
  if (action === "hold") updateIssue(id, { status: "held" }, "held for user decision.");
  if (action === "execution") {
    state.stage = "execute";
    state.activity = ["Variant B switched from review queue to execution board.", ...state.activity].slice(0, 8);
    render();
  }
  if (action === "review") {
    state.stage = "review";
    state.activity = ["Variant B returned to dependency-ordered review queue.", ...state.activity].slice(0, 8);
    render();
  }
  if (action === "export") {
    state.activity = ["Export summary preview requested. In production this would generate the HTML artifact.", ...state.activity].slice(0, 8);
    render();
  }
});

document.addEventListener("change", (event) => {
  const select = event.target.closest("[data-agent-for]");
  if (!select) return;
  overrideAgent(select.dataset.agentFor, select.value);
});

document.addEventListener("keydown", (event) => {
  const active = document.activeElement;
  if (active && ["INPUT", "TEXTAREA", "SELECT"].includes(active.tagName)) return;
  if (active && active.isContentEditable) return;
  if (!["ArrowLeft", "ArrowRight"].includes(event.key)) return;
  const current = getVariant();
  const index = VARIANTS.findIndex((variant) => variant.key === current);
  const delta = event.key === "ArrowRight" ? 1 : -1;
  const next = VARIANTS[(index + delta + VARIANTS.length) % VARIANTS.length];
  setVariant(next.key);
});

render();
