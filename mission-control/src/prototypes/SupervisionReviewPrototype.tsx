/**
 * PROTOTYPE — throw this away after resolving Wayfinder issue ISS-53.
 *
 * One supervision review inside the already-approved Variant C Focus desk.
 * All state is local display state; no Mission, runner, or backend is mutated.
 */
import { useMemo, useState } from "react";
import "./supervision-review-prototype.css";

type ScenarioKey =
  | "healthy"
  | "finished-unseen"
  | "runner-stopped"
  | "alfredo-restarted"
  | "facts-conflict";
type ConflictKey =
  | "invalid-result"
  | "pid-reused"
  | "probe-offline"
  | "terminal-live";
type ReviewStage = "before" | "after" | "receipt";
type DeliveryChoice = "atomic" | "outbox-first";
type RecoveryChoice = "commander-visible" | "automatic-after-proof";
type RestartEffectStep = "waiting" | "applied" | "replayed";
type Tone = "healthy" | "attention" | "danger" | "quiet";

interface Fact {
  readonly label: string;
  readonly detail: string;
  readonly tone: Tone;
}

interface ReviewFrame {
  readonly canonical: Fact;
  readonly observation: Fact;
  readonly attention: Fact;
  readonly outcome: Fact;
  readonly missionLine: string;
}

interface Scenario {
  readonly key: ScenarioKey;
  readonly label: string;
  readonly short: string;
  readonly question: string;
}

const SCENARIOS: readonly Scenario[] = [
  {
    key: "healthy",
    label: "Healthy",
    short: "Runner is alive and active",
    question: "Can healthy work stay completely quiet?",
  },
  {
    key: "finished-unseen",
    label: "Finished unseen",
    short: "A valid result exists",
    question: "Can Alfredo notice a missed completion without rerunning it?",
  },
  {
    key: "runner-stopped",
    label: "Runner stopped",
    short: "No owner and no result",
    question: "When is same-session recovery safe?",
  },
  {
    key: "alfredo-restarted",
    label: "Alfredo restarted",
    short: "Crash at the alert/bookmark cut",
    question: "Does restart replay create one alert and one effect?",
  },
  {
    key: "facts-conflict",
    label: "Facts conflict",
    short: "Evidence is unsafe or incomplete",
    question: "Does Alfredo fail closed instead of guessing?",
  },
] as const;

const CONFLICTS: ReadonlyArray<{
  readonly key: ConflictKey;
  readonly label: string;
}> = [
  { key: "invalid-result", label: "Bad result" },
  { key: "pid-reused", label: "Reused PID" },
  { key: "probe-offline", label: "Probe offline" },
  { key: "terminal-live", label: "Completed but live" },
] as const;

const BEFORE_FRAME: ReviewFrame = {
  canonical: {
    label: "Running · revision 12",
    detail:
      "The Orchestrator still owns the Mission/session record. That record is authoritative.",
    tone: "quiet",
  },
  observation: {
    label: "Check not run yet",
    detail:
      "Runner facts have not been joined with the canonical record in this step.",
    tone: "quiet",
  },
  attention: {
    label: "No operational attention",
    detail: "Mission Work has not projected a supervision finding.",
    tone: "quiet",
  },
  outcome: {
    label: "No action",
    detail: "An observation alone can never mutate Mission state.",
    tone: "quiet",
  },
  missionLine: "Running · no operational attention",
};

function conflictFrame(
  conflict: ConflictKey,
  stage: Exclude<ReviewStage, "before">,
): ReviewFrame {
  const cases: Record<ConflictKey, Omit<ReviewFrame, "canonical">> = {
    "invalid-result": {
      observation: {
        label: "Persisted result failed validation",
        detail:
          "Its content hash or exact session, operation, or worktree boundary does not match.",
        tone: "danger",
      },
      attention: {
        label: "Result validation failed",
        detail:
          "This outranks a dead-owner finding because rerunning could duplicate completed work.",
        tone: "danger",
      },
      outcome: {
        label: "Block completion and rerun",
        detail:
          "A person must inspect the result boundary. No recovery intent is created.",
        tone: "danger",
      },
      missionLine: "Running · 1 attention · result validation failed",
    },
    "pid-reused": {
      observation: {
        label: "PID belongs to another process",
        detail:
          "The number exists, but its Linux start identity differs and group absence is unknown.",
        tone: "danger",
      },
      attention: {
        label: "Runner identity is ambiguous",
        detail:
          "The replacement process is not accepted as the canonical Local Agent owner.",
        tone: "attention",
      },
      outcome: {
        label: "Inspect only",
        detail:
          "No completion or recovery is allowed until exact ownership and quiescence are proven.",
        tone: "danger",
      },
      missionLine: "Running · 1 attention · runner identity ambiguous",
    },
    "probe-offline": {
      observation: {
        label: "Independent check unavailable",
        detail:
          "Runner and process-group facts remain unavailable beyond the selected grace period.",
        tone: "attention",
      },
      attention: {
        label: "Liveness unavailable",
        detail:
          "The alert remains visible across Alfredo restart; unavailable never means finished.",
        tone: "attention",
      },
      outcome: {
        label: "Wait or inspect",
        detail: "No success, failure, or rerun is inferred from missing evidence.",
        tone: "attention",
      },
      missionLine: "Running · 1 attention · liveness unavailable",
    },
    "terminal-live": {
      observation: {
        label: "Exact runner is still live",
        detail:
          "Canonical Mission state is terminal, but runner/process-group quiescence is not proven.",
        tone: "danger",
      },
      attention: {
        label: "Runner still live",
        detail:
          "Completed remains authoritative, but retirement cannot begin until the runner owner and process group are quiescent.",
        tone: "attention",
      },
      outcome: {
        label: "Wait for quiescence",
        detail:
          "Do not relabel the work or retire its session/worktree while either remains live.",
        tone: "attention",
      },
      missionLine: "Completed · 1 attention · runner still live",
    },
  };
  const selected = cases[conflict];
  return {
    canonical: {
      label:
        conflict === "terminal-live"
          ? "Completed · revision 13"
          : "Running · revision 12",
      detail:
        "Canonical Mission/session state keeps authority even when advisory facts disagree.",
      tone: conflict === "terminal-live" ? "healthy" : "quiet",
    },
    ...selected,
    outcome:
      stage === "receipt"
        ? {
            ...selected.outcome,
            label: "No effect receipt issued",
            detail: `${selected.outcome.detail} Restart does not weaken this boundary.`,
          }
        : selected.outcome,
  };
}

function reviewFrame(
  scenario: ScenarioKey,
  stage: ReviewStage,
  conflict: ConflictKey,
  delivery: DeliveryChoice,
  recovery: RecoveryChoice,
  restartEffectStep: RestartEffectStep,
): ReviewFrame {
  if (stage === "before") {
    if (scenario === "facts-conflict" && conflict === "terminal-live") {
      return {
        ...BEFORE_FRAME,
        canonical: {
          label: "Completed · revision 13",
          detail:
            "The canonical completion receipt already exists before the independent owner check is processed.",
          tone: "healthy",
        },
        missionLine: "Supervision degraded · 1 observation pending",
      };
    }
    return BEFORE_FRAME;
  }

  if (scenario === "healthy") {
    return {
      canonical: {
        label: "Running · revision 12",
        detail: "The canonical session remains Running; no revision is written.",
        tone: "healthy",
      },
      observation: {
        label: "Exact owner is live and active",
        detail:
          "PID, start identity, process group, operation, and worktree all match.",
        tone: "healthy",
      },
      attention: {
        label: "No attention",
        detail:
          "Five checks can perform 15 bounded probes with zero model turns, UI noise, or journal entries.",
        tone: "healthy",
      },
      outcome: {
        label: "No receipt needed",
        detail: "No-change work remains silent and token-free.",
        tone: "healthy",
      },
      missionLine: "Running · no operational attention",
    };
  }

  if (scenario === "finished-unseen") {
    const afterReceipt = stage === "receipt";
    return {
      canonical: {
        label: afterReceipt
          ? "Evidence ready · revision 13"
          : "Running · revision 12",
        detail: afterReceipt
          ? "Only the typed result-reconciliation receipt advanced canonical state."
          : "The runner's normal completion callback never updated this record.",
        tone: afterReceipt ? "healthy" : "attention",
      },
      observation: {
        label: "Exact valid result; owner and group absent",
        detail:
          "Result hash, Mission, session, operation, and worktree all validate.",
        tone: "healthy",
      },
      attention: {
        label: afterReceipt ? "Resolved with receipt" : "Result ready to reconcile",
        detail: afterReceipt
          ? "The durable incident remains inspectable; it is not emitted twice."
          : "Persisted work takes precedence over any dead-owner recovery.",
        tone: afterReceipt ? "healthy" : "attention",
      },
      outcome: {
        label: afterReceipt ? "Result receipt applied once" : "Reconcile result",
        detail: afterReceipt
          ? "The same session becomes Evidence ready; no rerun occurs."
          : "A typed Orchestrator action validates the exact result before mutation.",
        tone: "healthy",
      },
      missionLine: afterReceipt
        ? "Evidence ready · no operational attention"
        : "Running · 1 attention · result ready to reconcile",
    };
  }

  if (scenario === "runner-stopped") {
    const automatic = recovery === "automatic-after-proof";
    const applied = automatic || stage === "receipt";
    const receiptReplay = automatic && stage === "receipt";
    return {
      canonical: {
        label: applied ? "Queued · revision 13" : "Running · revision 12",
        detail: applied
          ? "A typed same-session recovery receipt queued the existing worktree once."
          : "The observation does not directly change canonical state.",
        tone: applied ? "healthy" : "attention",
      },
      observation: {
        label: "Exact owner and group absent; no result",
        detail:
          "Mission, session, revision, worktree, PID/start, group, and operation boundaries all match.",
        tone: "attention",
      },
      attention: {
        label: applied
          ? receiptReplay
            ? "Recovery receipt replayed"
            : "Recovery receipt recorded"
          : "Same-session recovery ready",
        detail: applied
          ? "The incident is resolved by one receipt; replay cannot queue a second recovery."
          : "Repeated detectors merge into this one durable incident.",
        tone: applied ? "healthy" : "attention",
      },
      outcome: {
        label: applied
          ? receiptReplay
            ? "Same receipt; no second effect"
            : automatic
              ? "Queued automatically after exact proof"
              : "Approved and queued once"
          : "Ask before recovery",
        detail: applied
          ? "The effect rechecked the complete identity, quiescence, and result-absence boundary."
          : "Show one governed action; do nothing until you approve it.",
        tone: applied ? "healthy" : "attention",
      },
      missionLine: applied
        ? "Queued · infrastructure recovery acknowledged"
        : "Running · 1 attention · same-session recovery ready",
    };
  }

  if (scenario === "alfredo-restarted") {
    const afterRestart = stage === "receipt";
    const atomic = delivery === "atomic";
    const effectApplied =
      afterRestart &&
      (recovery === "automatic-after-proof" ||
        restartEffectStep === "applied" ||
        restartEffectStep === "replayed");
    const effectReplayed =
      afterRestart && restartEffectStep === "replayed";
    return {
      canonical: {
        label: effectApplied ? "Queued · revision 13" : "Running · revision 12",
        detail: effectApplied
          ? "One typed same-session recovery receipt queued the existing worktree."
          : "No observation or watcher restart has changed the Mission/session record.",
        tone: effectApplied ? "healthy" : "quiet",
      },
      observation: {
        label: afterRestart
          ? "Event replayed from cursor 0/1"
          : "Watcher crashed at the alert/bookmark cut",
        detail: afterRestart
          ? "The same semantic incident is re-read after restart."
          : atomic
            ? "The all-or-nothing ledger write aborted; neither alert nor cursor was saved."
            : "ATT-001 is durable first; the cursor bookmark remains at 0/1.",
        tone: "attention",
      },
      attention: {
        label: effectApplied
          ? effectReplayed
            ? "Same recovery receipt replayed"
            : "One recovery receipt recorded"
          : afterRestart
            ? "One dead-owner incident delivered"
          : atomic
            ? "No partial record"
            : "ATT-001 saved · cursor pending",
        detail: effectApplied
          ? "The incident is resolved; replay retains the same correlation and receipt."
          : afterRestart
            ? "Cursor advances to 1/1 and the open incident retains one correlation id."
          : atomic
            ? "Restart safely retries the whole event."
            : "Restart sees the saved alert before moving the cursor.",
        tone: effectApplied ? "healthy" : "attention",
      },
      outcome: {
        label: effectApplied
          ? effectReplayed
            ? "Duplicate effect prevented"
            : recovery === "automatic-after-proof"
              ? "Queued automatically after restart proof"
              : "Approved and queued once"
          : afterRestart
            ? "Recovery ready; no effect yet"
            : "Restart watcher",
        detail: effectApplied
          ? "Replaying the same typed request returns one receipt and cannot queue again."
          : afterRestart
            ? "A typed recovery still requires the exact boundary and the selected recovery policy."
            : atomic
              ? "Restart restores the durable source event and unchanged cursor for a full retry."
              : "Restart retains the durable attention while the unchanged cursor replays the event.",
        tone: effectApplied ? "healthy" : "attention",
      },
      missionLine: effectApplied
        ? "Queued · infrastructure recovery acknowledged"
        : afterRestart
          ? "Running · 1 attention · dead owner"
          : atomic
            ? "Supervision degraded · 1 observation pending"
            : "Running · 1 attention · dead owner · supervision backlog 1",
    };
  }

  return conflictFrame(conflict, stage);
}

function ChoiceButton({
  active,
  label,
  detail,
  recommended = false,
  onClick,
}: {
  readonly active: boolean;
  readonly label: string;
  readonly detail: string;
  readonly recommended?: boolean;
  readonly onClick: () => void;
}) {
  return (
    <button
      aria-pressed={active}
      className="met-supervision-choice"
      data-active={active || undefined}
      onClick={onClick}
      type="button"
    >
      <span>
        <strong>{label}</strong>
        {recommended ? <small>Recommended starting point</small> : null}
      </span>
      <p>{detail}</p>
    </button>
  );
}

function TimingControl({
  label,
  value,
  unit,
  onDecrease,
  onIncrease,
}: {
  readonly label: string;
  readonly value: number;
  readonly unit: string;
  readonly onDecrease: () => void;
  readonly onIncrease: () => void;
}) {
  return (
    <div className="met-supervision-timing">
      <span>{label}</span>
      <div>
        <button aria-label={`Decrease ${label}`} onClick={onDecrease} type="button">
          −
        </button>
        <strong>
          {value}
          {unit}
        </strong>
        <button aria-label={`Increase ${label}`} onClick={onIncrease} type="button">
          +
        </button>
      </div>
    </div>
  );
}

export function SupervisionReviewPrototype() {
  const [scenarioKey, setScenarioKey] =
    useState<ScenarioKey>("finished-unseen");
  const [conflictKey, setConflictKey] =
    useState<ConflictKey>("invalid-result");
  const [stage, setStage] = useState<ReviewStage>("after");
  const [delivery, setDelivery] = useState<DeliveryChoice>("atomic");
  const [recovery, setRecovery] =
    useState<RecoveryChoice>("commander-visible");
  const [restartEffectStep, setRestartEffectStep] =
    useState<RestartEffectStep>("waiting");
  const [cadenceSeconds, setCadenceSeconds] = useState(15);
  const [staleSweeps, setStaleSweeps] = useState(3);
  const [unavailableSweeps, setUnavailableSweeps] = useState(2);
  const [summaryReady, setSummaryReady] = useState(false);

  const scenario =
    SCENARIOS.find((candidate) => candidate.key === scenarioKey) ?? SCENARIOS[0];
  const staleSeconds = cadenceSeconds * staleSweeps;
  const unavailableSeconds = cadenceSeconds * unavailableSweeps;
  const frame = useMemo(
    () =>
      reviewFrame(
        scenarioKey,
        stage,
        conflictKey,
        delivery,
        recovery,
        restartEffectStep,
      ),
    [
      conflictKey,
      delivery,
      recovery,
      restartEffectStep,
      scenarioKey,
      stage,
    ],
  );
  const deliverySummary =
    delivery === "atomic"
      ? "require an atomic alert-and-cursor ledger"
      : "use atomic persistence plus an attention-first fallback";
  const recoverySummary =
    recovery === "commander-visible"
      ? "ask before same-session recovery"
      : "recover automatically only after exact proof";
  const receiptStageLabel =
    scenarioKey === "healthy"
      ? "After another check"
      : scenarioKey === "finished-unseen"
        ? "After result receipt"
        : scenarioKey === "runner-stopped"
          ? recovery === "automatic-after-proof"
            ? "After receipt replay"
            : "After approval receipt"
          : scenarioKey === "alfredo-restarted"
            ? "After restart"
            : "After restart";

  return (
    <section
      aria-label="Attention-driven Local Agent supervision review"
      className="met-supervision-review"
    >
      <header className="met-supervision-review__header">
        <div>
          <span>ISS-53 · PROTOTYPE CASES · NO MISSION STATE CHANGES</span>
          <h3>When a Local Agent goes quiet, what should Alfredo do?</h3>
          <p>
            Choose a situation. The four cards keep the authoritative Mission
            record separate from advisory checks, visible attention, and the
            only allowed Orchestrator outcome.
          </p>
        </div>
        <strong>Decision needed</strong>
      </header>

      <nav aria-label="Supervision scenarios" className="met-supervision-scenarios">
        {SCENARIOS.map((candidate) => (
          <button
            aria-pressed={scenarioKey === candidate.key}
            data-active={scenarioKey === candidate.key || undefined}
            key={candidate.key}
            onClick={() => {
              setScenarioKey(candidate.key);
              setStage("after");
              setRestartEffectStep("waiting");
              setSummaryReady(false);
            }}
            type="button"
          >
            <strong>{candidate.label}</strong>
            <span>{candidate.short}</span>
          </button>
        ))}
      </nav>

      {scenarioKey === "facts-conflict" ? (
        <div
          aria-label="Conflicting fact case"
          className="met-supervision-conflicts"
          role="group"
        >
          {CONFLICTS.map((conflict) => (
            <button
              aria-pressed={conflictKey === conflict.key}
              data-active={conflictKey === conflict.key || undefined}
              key={conflict.key}
              onClick={() => {
                setConflictKey(conflict.key);
                setSummaryReady(false);
              }}
              type="button"
            >
              {conflict.label}
            </button>
          ))}
        </div>
      ) : null}

      <section className="met-supervision-case">
        <header>
          <div>
            <span>CASE QUESTION</span>
            <strong>{scenario.question}</strong>
          </div>
          <div
            aria-label="Scenario stage"
            className="met-supervision-stages"
            role="group"
          >
            {(
              [
                ["before", "Before check"],
                ["after", "After check"],
                ["receipt", receiptStageLabel],
              ] as const
            ).map(([key, label]) => (
              <button
                aria-pressed={stage === key}
                data-active={stage === key || undefined}
                key={key}
                onClick={() => setStage(key)}
                type="button"
              >
                {label}
              </button>
            ))}
          </div>
        </header>

        <div className="met-supervision-flow">
          {(
            [
              ["1", "Canonical Mission record", frame.canonical],
              ["2", "Independent check", frame.observation],
              ["3", "Mission Work shows", frame.attention],
              ["4", "Allowed outcome", frame.outcome],
            ] as const
          ).map(([step, title, fact]) => (
            <article data-tone={fact.tone} key={title}>
              <header>
                <span>{step}</span>
                <strong>{title}</strong>
              </header>
              <h4>{fact.label}</h4>
              <p>{fact.detail}</p>
            </article>
          ))}
        </div>

        <output className="met-supervision-mission-line">
          <span>WHAT VARIANT C WOULD SHOW</span>
          <strong>{frame.missionLine}</strong>
        </output>
        {scenarioKey === "alfredo-restarted" && stage === "receipt" ? (
          <div className="met-supervision-effect-replay">
            <span>
              {recovery === "automatic-after-proof" ||
              restartEffectStep !== "waiting"
                ? "One typed recovery receipt exists."
                : "Restart delivered one attention; no recovery effect exists yet."}
            </span>
            {recovery === "commander-visible" &&
            restartEffectStep === "waiting" ? (
              <button
                onClick={() => setRestartEffectStep("applied")}
                type="button"
              >
                Approve typed recovery
              </button>
            ) : restartEffectStep !== "replayed" ? (
              <button
                onClick={() => setRestartEffectStep("replayed")}
                type="button"
              >
                Replay same effect request
              </button>
            ) : (
              <strong>Same receipt returned · second effect prevented</strong>
            )}
          </div>
        ) : null}
      </section>

      <section className="met-supervision-policies">
        <header>
          <span>YOUR REVIEW CHOICES</span>
          <h3>Choose the behavior, then try the cases again</h3>
          <p>
            These controls change only this page. They make the policy trade-offs
            visible before anything is specified or implemented.
          </p>
        </header>

        <fieldset>
          <legend>1 · Persistence support for alerts and observer cursors</legend>
          <div className="met-supervision-choice-grid">
            <ChoiceButton
              active={delivery === "atomic"}
              detail="Require one atomic commit. An actionable cursor receipt can commit only when it references durable attention and, when effectful, that attention's typed intent."
              label="Atomic stores only"
              onClick={() => {
                setDelivery("atomic");
                setRestartEffectStep("waiting");
                setSummaryReady(false);
              }}
              recommended
            />
            <ChoiceButton
              active={delivery === "outbox-first"}
              detail="Use atomic commits where available; otherwise save durable attention and any effectful typed intent first, advance the cursor second, and merge once after restart."
              label="Atomic + attention-first fallback"
              onClick={() => {
                setDelivery("outbox-first");
                setRestartEffectStep("waiting");
                setSummaryReady(false);
              }}
            />
          </div>
        </fieldset>

        <fieldset>
          <legend>2 · After exact stopped-runner and no-result proof</legend>
          <div className="met-supervision-choice-grid">
            <ChoiceButton
              active={recovery === "commander-visible"}
              detail="Show one governed same-session recovery action and wait for your approval."
              label="Ask me first"
              onClick={() => {
                setRecovery("commander-visible");
                setRestartEffectStep("waiting");
                setSummaryReady(false);
              }}
              recommended
            />
            <ChoiceButton
              active={recovery === "automatic-after-proof"}
              detail="Queue the same session/worktree once without a click, but only after every exact boundary matches."
              label="Recover automatically"
              onClick={() => {
                setRecovery("automatic-after-proof");
                setRestartEffectStep("waiting");
                setSummaryReady(false);
              }}
            />
          </div>
        </fieldset>

        <fieldset>
          <legend>3 · How quickly supervision becomes visible</legend>
          <div className="met-supervision-timing-grid">
            <TimingControl
              label="Check every"
              onDecrease={() => {
                setCadenceSeconds((value) => Math.max(5, value - 5));
                setSummaryReady(false);
              }}
              onIncrease={() => {
                setCadenceSeconds((value) => Math.min(60, value + 5));
                setSummaryReady(false);
              }}
              unit="s"
              value={cadenceSeconds}
            />
            <TimingControl
              label="Show stale after"
              onDecrease={() => {
                setStaleSweeps((value) => Math.max(1, value - 1));
                setSummaryReady(false);
              }}
              onIncrease={() => {
                setStaleSweeps((value) => Math.min(12, value + 1));
                setSummaryReady(false);
              }}
              unit={` checks · ${staleSeconds}s`}
              value={staleSweeps}
            />
            <TimingControl
              label="Show unavailable after"
              onDecrease={() => {
                setUnavailableSweeps((value) => Math.max(1, value - 1));
                setSummaryReady(false);
              }}
              onIncrease={() => {
                setUnavailableSweeps((value) => Math.min(12, value + 1));
                setSummaryReady(false);
              }}
              unit={` checks · ${unavailableSeconds}s`}
              value={unavailableSweeps}
            />
          </div>
        </fieldset>
      </section>

      <section className="met-supervision-safety">
        <header>
          <span>FIXED SAFETY RULES SHOWN IN EVERY CASE</span>
          <strong>Observations advise; the Orchestrator remains authoritative</strong>
        </header>
        <ul>
          <li>Repeated detectors merge into one semantic incident.</li>
          <li>
            An actionable cursor receipt always references durable attention and,
            when effectful, that attention&apos;s typed intent.
          </li>
          <li>Resolved or superseded incidents keep their typed receipt.</li>
          <li>Invalid or unreconciled results block duplicate reruns.</li>
          <li>
            Effects recheck Mission, session, revision, worktree, PID/start,
            process group, operation, quiescence, and result boundaries.
          </li>
        </ul>
      </section>

      <details className="met-supervision-evidence">
        <summary>Engineering evidence behind these five review cases</summary>
        <p>
          The separate reducer exercises 23 deterministic fault comparisons and
          eight explicit invariants. An independent audit also ran 10,000
          randomized 24-action sequences across both policy choices with no
          exception or invariant failure. Healthy checks used zero model turns.
        </p>
      </details>

      <footer className="met-supervision-summary">
        <div>
          <span>PROTOTYPE REVIEW SUMMARY</span>
          <p>
            {deliverySummary}; {recoverySummary}; check every {cadenceSeconds}s,
            show stale after {staleSweeps} checks ({staleSeconds}s), and show
            unavailable after {unavailableSweeps} checks ({unavailableSeconds}s);
            group duplicate reports and retain resolution receipts.
          </p>
        </div>
        <button onClick={() => setSummaryReady(true)} type="button">
          Prepare review summary
        </button>
        {summaryReady ? (
          <output>
            Tell Codex <strong>“Approve as shown”</strong>, or describe what you
            want changed. No backend or Mission state was changed.
          </output>
        ) : null}
      </footer>
    </section>
  );
}
