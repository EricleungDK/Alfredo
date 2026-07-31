/**
 * PROTOTYPE ONLY — three placements for a live Rust shadow review inside the
 * current Alfredo workstation, switchable with ?variant=A|B|C.
 *
 * The current React/Tauri/Python workstation remains the visible canonical
 * product. Every prototype action executes in Rust through a Tauri command,
 * stays in memory, reloads Python before and after, and performs zero writes.
 */
import { invoke } from "@tauri-apps/api/core";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from "react";
import "./rust-orchestrator-gui-prototype.css";

type JourneyPhase =
  | "selection-required"
  | "mission-choice-required"
  | "ready-for-formation"
  | "planning-required"
  | "dispatch-ready"
  | "session-queued";

type FormationRoute =
  | "no-action"
  | "ad-hoc-delegation"
  | "bounded-discovery"
  | "wayfinding";

interface MissionRef {
  readonly id: string;
  readonly title: string;
}

interface EffectReceipt {
  readonly correlation_id: string;
  readonly revision: number;
  readonly effect_kind: string;
  readonly canonical_effect: boolean;
  readonly replayed: boolean;
  readonly session_id: string | null;
  readonly message: string;
}

interface PrototypeState {
  readonly schema_version: number;
  readonly revision: number;
  readonly starting_location: string;
  readonly coding_workspace: string | null;
  readonly phase: JourneyPhase;
  readonly known_missions: readonly MissionRef[];
  readonly active_mission_id: string | null;
  readonly formation_route: FormationRoute | null;
  readonly task: string | null;
  readonly sessions: readonly {
    readonly session_id: string;
    readonly mission_id: string;
    readonly task: string;
    readonly status: string;
  }[];
  readonly receipts: Readonly<Record<string, EffectReceipt>>;
}

interface PythonAuthority {
  readonly revision: number;
  readonly workspace_path: string;
  readonly workspace_status: string;
  readonly active_mission_id: string | null;
  readonly active_mission_title: string | null;
}

type PrototypeAction =
  | {
      readonly action: "select-workspace";
      readonly correlation_id: string;
      readonly expected_revision: number;
      readonly path: string;
      readonly repository_valid: boolean;
      readonly known_missions: readonly MissionRef[];
    }
  | {
      readonly action: "choose-mission";
      readonly correlation_id: string;
      readonly expected_revision: number;
      readonly choice:
        | { readonly choice: "resume"; readonly mission_id: string }
        | {
            readonly choice: "start-new";
            readonly mission_id: string;
            readonly title: string;
          };
    }
  | {
      readonly action: "form-mission";
      readonly correlation_id: string;
      readonly expected_revision: number;
      readonly route: FormationRoute;
      readonly task: string;
      readonly effectful_request: boolean;
      readonly controller_claims_effect: boolean;
    }
  | {
      readonly action: "dispatch";
      readonly correlation_id: string;
      readonly expected_revision: number;
      readonly proposal_exact: boolean;
      readonly worker_eligible: boolean;
    };

interface PrototypeResponse {
  readonly schema_version: 1;
  readonly mode: "rust-shadow";
  readonly state: PrototypeState;
  readonly receipt: EffectReceipt | null;
  readonly python_authority: PythonAuthority;
  readonly python_unchanged_during_request: boolean;
  readonly canonical_writes_performed: false;
  readonly elapsed_micros: number;
  readonly message: string;
}

interface TraceEntry {
  readonly id: number;
  readonly tone: "rust" | "python" | "warning";
  readonly label: string;
  readonly detail: string;
  readonly elapsedMicros: number;
}

interface ReviewModel {
  readonly state: PrototypeState | null;
  readonly python: PythonAuthority | null;
  readonly baseline: PythonAuthority | null;
  readonly receipt: EffectReceipt | null;
  readonly trace: readonly TraceEntry[];
  readonly pending: boolean;
  readonly error: string;
  readonly lastAction: PrototypeAction | null;
  readonly goal: string;
  readonly route: FormationRoute;
  readonly setGoal: (goal: string) => void;
  readonly setRoute: (route: FormationRoute) => void;
  readonly load: () => Promise<void>;
  readonly resetSelection: () => Promise<void>;
  readonly selectWorkspace: () => Promise<void>;
  readonly resumeMission: () => Promise<void>;
  readonly applyRoute: () => Promise<void>;
  readonly dispatch: () => Promise<void>;
  readonly replay: () => Promise<void>;
  readonly falseSuccess: () => Promise<void>;
  readonly rollback: () => Promise<void>;
}

const routeLabels: Record<FormationRoute, string> = {
  "no-action": "No action",
  "ad-hoc-delegation": "Ad Hoc Delegation",
  "bounded-discovery": "Bounded discovery",
  wayfinding: "Wayfinding",
};

function sameAuthority(left: PythonAuthority | null, right: PythonAuthority | null) {
  return (
    left !== null &&
    right !== null &&
    left.revision === right.revision &&
    left.workspace_path === right.workspace_path &&
    left.workspace_status === right.workspace_status &&
    left.active_mission_id === right.active_mission_id
  );
}

function useRustReviewModel(): ReviewModel {
  const [state, setState] = useState<PrototypeState | null>(null);
  const [python, setPython] = useState<PythonAuthority | null>(null);
  const [baseline, setBaseline] = useState<PythonAuthority | null>(null);
  const [receipt, setReceipt] = useState<EffectReceipt | null>(null);
  const [trace, setTrace] = useState<readonly TraceEntry[]>([]);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const [lastAction, setLastAction] = useState<PrototypeAction | null>(null);
  const [goal, setGoal] = useState("Add one bounded receipt seam");
  const [route, setRoute] = useState<FormationRoute>("ad-hoc-delegation");
  const [sequence, setSequence] = useState(1);
  const initialLoadStarted = useRef(false);

  const call = useCallback(
    async (operation: string, action: PrototypeAction | null = null) => {
      setPending(true);
      setError("");
      try {
        const response = await invoke<PrototypeResponse>("rust_orchestrator_prototype", {
          request: { operation, state, action },
        });
        setState(response.state);
        setPython(response.python_authority);
        setBaseline((current) => current ?? response.python_authority);
        setReceipt(response.receipt);
        if (action) {
          setLastAction(action);
        } else if (operation === "load" || operation === "rollback") {
          setLastAction(null);
        }
        const entry: TraceEntry = {
          id: trace.length + 1,
          tone: response.python_unchanged_during_request ? "rust" : "warning",
          label:
            response.receipt?.effect_kind ??
            (operation === "rollback" ? "python-restored" : operation),
          detail: response.message,
          elapsedMicros: response.elapsed_micros,
        };
        setTrace((current) =>
          [{ ...entry, id: current.length + 1 }, ...current].slice(0, 8),
        );
        return response;
      } catch (reason) {
        const message =
          reason instanceof Error ? reason.message : String(reason ?? "Unknown Rust failure");
        setError(message);
        return null;
      } finally {
        setPending(false);
      }
    },
    [state, trace.length],
  );

  const correlation = useCallback(
    (kind: string) => {
      const value = `rust-gui-${kind}-${sequence}`;
      setSequence((current) => current + 1);
      return value;
    },
    [sequence],
  );

  const load = useCallback(async () => {
    await call("load");
  }, [call]);

  useEffect(() => {
    if (initialLoadStarted.current) return;
    initialLoadStarted.current = true;
    void load();
  }, [load]);

  const resetSelection = useCallback(async () => {
    await call("reset-selection");
  }, [call]);

  const selectWorkspace = useCallback(async () => {
    if (!state || !python) return;
    await call("apply", {
      action: "select-workspace",
      correlation_id: correlation("workspace"),
      expected_revision: state.revision,
      path: python.workspace_path,
      repository_valid: true,
      known_missions:
        python.active_mission_id && python.active_mission_title
          ? [
              {
                id: python.active_mission_id,
                title: python.active_mission_title,
              },
            ]
          : [],
    });
  }, [call, correlation, python, state]);

  const resumeMission = useCallback(async () => {
    if (!state || !python?.active_mission_id) return;
    await call("apply", {
      action: "choose-mission",
      correlation_id: correlation("mission"),
      expected_revision: state.revision,
      choice: {
        choice: "resume",
        mission_id: python.active_mission_id,
      },
    });
  }, [call, correlation, python, state]);

  const applyRoute = useCallback(async () => {
    if (!state || !goal.trim()) return;
    await call("apply", {
      action: "form-mission",
      correlation_id: correlation("route"),
      expected_revision: state.revision,
      route,
      task: goal.trim(),
      effectful_request: route !== "no-action",
      controller_claims_effect: false,
    });
  }, [call, correlation, goal, route, state]);

  const dispatch = useCallback(async () => {
    if (!state) return;
    await call("apply", {
      action: "dispatch",
      correlation_id: correlation("dispatch"),
      expected_revision: state.revision,
      proposal_exact: true,
      worker_eligible: true,
    });
  }, [call, correlation, state]);

  const replay = useCallback(async () => {
    if (!lastAction) return;
    await call("apply", lastAction);
  }, [call, lastAction]);

  const falseSuccess = useCallback(async () => {
    if (!state) return;
    await call("apply", {
      action: "form-mission",
      correlation_id: correlation("false-success"),
      expected_revision: state.revision,
      route: "no-action",
      task: "Yes, create the requested folder now",
      effectful_request: true,
      controller_claims_effect: true,
    });
  }, [call, correlation, state]);

  const rollback = useCallback(async () => {
    await call("rollback");
  }, [call]);

  return {
    state,
    python,
    baseline,
    receipt,
    trace,
    pending,
    error,
    lastAction,
    goal,
    route,
    setGoal,
    setRoute,
    load,
    resetSelection,
    selectWorkspace,
    resumeMission,
    applyRoute,
    dispatch,
    replay,
    falseSuccess,
    rollback,
  };
}

function SafetyStrip({ model }: { readonly model: ReviewModel }) {
  const ready = model.baseline !== null && model.python !== null;
  const unchanged = sameAuthority(model.baseline, model.python);
  return (
    <div className="rust-flight-safety" role="status" aria-live="polite">
      <span className="rust-flight-safety__light" data-safe={ready ? unchanged : undefined} />
      <strong>
        {!ready
          ? "Reading Python authority"
          : unchanged
            ? "Python authority unchanged"
            : "Python authority changed"}
      </strong>
      <span>Rust writes: 0</span>
      <span>Rollback: live</span>
      {model.pending && <span className="rust-flight-safety__pulse">Rust evaluating…</span>}
    </div>
  );
}

function JourneyTrack({ state }: { readonly state: PrototypeState | null }) {
  const current = state?.phase ?? "selection-required";
  const stages: readonly [JourneyPhase, string][] = [
    ["selection-required", "Starting Location"],
    ["mission-choice-required", "Coding Workspace"],
    ["ready-for-formation", "Mission choice"],
    ["dispatch-ready", "Formation route"],
    ["session-queued", "Queued receipt"],
  ];
  const currentIndex = Math.max(
    0,
    stages.findIndex(([phase]) => phase === current),
  );
  return (
    <ol className="rust-flight-track" aria-label="Rust shadow journey">
      {stages.map(([phase, label], index) => (
        <li
          key={phase}
          data-current={phase === current}
          data-complete={index < currentIndex}
        >
          <span>{String(index + 1).padStart(2, "0")}</span>
          <strong>{label}</strong>
        </li>
      ))}
    </ol>
  );
}

function ActionDeck({ model, compact = false }: { readonly model: ReviewModel; compact?: boolean }) {
  const phase = model.state?.phase;
  return (
    <section className="rust-flight-actions" data-compact={compact}>
      <header>
        <span className="rust-flight-kicker">Typed Rust controls</span>
        <strong>{phase?.replaceAll("-", " ") ?? "Loading live state"}</strong>
      </header>
      <div className="rust-flight-actions__row">
        <button type="button" onClick={() => void model.resetSelection()} disabled={model.pending}>
          Reset to Starting Location
        </button>
        <button
          type="button"
          onClick={() => void model.selectWorkspace()}
          disabled={model.pending || phase !== "selection-required"}
        >
          Select live workspace
        </button>
        <button
          type="button"
          onClick={() => void model.resumeMission()}
          disabled={
            model.pending ||
            phase !== "mission-choice-required" ||
            !model.python?.active_mission_id
          }
        >
          Resume live Mission
        </button>
      </div>
      <button
        type="button"
        className="rust-flight-rollback"
        onClick={() => void model.rollback()}
        disabled={model.pending}
      >
        Discard Rust state · return to live Python
      </button>
      <label>
        Goal
        <input
          value={model.goal}
          onChange={(event) => model.setGoal(event.target.value)}
          disabled={model.pending}
        />
      </label>
      <div className="rust-flight-actions__route">
        <label>
          Mission Formation Route
          <select
            value={model.route}
            onChange={(event) => model.setRoute(event.target.value as FormationRoute)}
            disabled={model.pending}
          >
            {Object.entries(routeLabels).map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          onClick={() => void model.applyRoute()}
          disabled={model.pending || phase !== "ready-for-formation" || !model.goal.trim()}
        >
          Apply route in Rust
        </button>
      </div>
      <div className="rust-flight-actions__row">
        <button
          type="button"
          className="rust-flight-primary"
          onClick={() => void model.dispatch()}
          disabled={model.pending || phase !== "dispatch-ready"}
        >
          Queue exact shadow session
        </button>
        <button
          type="button"
          onClick={() => void model.replay()}
          disabled={model.pending || !model.lastAction}
        >
          Replay last receipt
        </button>
        <button
          type="button"
          onClick={() => void model.falseSuccess()}
          disabled={model.pending || phase !== "ready-for-formation"}
        >
          Try false-success
        </button>
      </div>
      {model.error && <p className="rust-flight-error">{model.error}</p>}
    </section>
  );
}

function AuthorityCard({
  kind,
  title,
  rows,
}: {
  readonly kind: "python" | "rust";
  readonly title: string;
  readonly rows: readonly [string, ReactNode][];
}) {
  return (
    <section className="rust-flight-authority" data-kind={kind}>
      <header>
        <span>{kind === "python" ? "PY" : "RS"}</span>
        <div>
          <strong>{title}</strong>
          <small>{kind === "python" ? "canonical writer" : "in-memory candidate"}</small>
        </div>
      </header>
      <dl>
        {rows.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value ?? "—"}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

function ComparisonDeck({ model }: { readonly model: ReviewModel }) {
  const state = model.state;
  const python = model.python;
  return (
    <div className="rust-flight-comparison">
      <AuthorityCard
        kind="python"
        title="Live Python authority"
        rows={[
          ["Revision", python?.revision ?? "Loading"],
          ["Workspace", python?.workspace_path ?? "Loading"],
          ["Mission", python?.active_mission_title ?? "None"],
          ["Status", python?.workspace_status ?? "Loading"],
          ["Writes", "Canonical"],
        ]}
      />
      <div className="rust-flight-bridge" aria-label="Shadow comparison bridge">
        <span>READ V1</span>
        <i aria-hidden="true">→</i>
        <strong>COMPARE</strong>
        <i aria-hidden="true">←</i>
        <span>0 WRITES</span>
      </div>
      <AuthorityCard
        kind="rust"
        title="Rust shadow"
        rows={[
          ["Revision", state?.revision ?? "Loading"],
          ["Workspace", state?.coding_workspace ?? "Selection required"],
          ["Mission", state?.active_mission_id ?? "None"],
          ["Phase", state?.phase ?? "Loading"],
          ["Sessions", state?.sessions.length ?? 0],
        ]}
      />
    </div>
  );
}

function ReceiptCard({ receipt }: { readonly receipt: EffectReceipt | null }) {
  return (
    <section className="rust-flight-receipt">
      <header>
        <span>Latest Rust receipt</span>
        <strong>{receipt?.effect_kind ?? "No effect evaluated"}</strong>
      </header>
      {receipt ? (
        <>
          <p>{receipt.message}</p>
          <dl>
            <div>
              <dt>Correlation</dt>
              <dd>{receipt.correlation_id}</dd>
            </div>
            <div>
              <dt>Canonical effect</dt>
              <dd>{receipt.canonical_effect ? "Candidate effect" : "No action"}</dd>
            </div>
            <div>
              <dt>Replay</dt>
              <dd>{receipt.replayed ? "Exact receipt replayed" : "First evaluation"}</dd>
            </div>
          </dl>
        </>
      ) : (
        <p>Choose a journey action to see the exact typed outcome.</p>
      )}
    </section>
  );
}

function TraceLedger({ trace }: { readonly trace: readonly TraceEntry[] }) {
  return (
    <section className="rust-flight-trace">
      <header>
        <span>Bridge flight recorder</span>
        <small>{trace.length} recent evaluations</small>
      </header>
      <ol>
        {trace.map((entry) => (
          <li key={entry.id} data-tone={entry.tone}>
            <span>{String(entry.id).padStart(2, "0")}</span>
            <div>
              <strong>{entry.label}</strong>
              <p>{entry.detail}</p>
            </div>
            <small>{(entry.elapsedMicros / 1000).toFixed(1)} ms</small>
          </li>
        ))}
      </ol>
    </section>
  );
}

function RailVariant({
  model,
  close,
}: {
  readonly model: ReviewModel;
  readonly close: () => void;
}) {
  return (
    <aside className="rust-flight rust-flight--rail" aria-label="Rust shadow review rail">
      <PrototypeHeader name="A · Shadow rail" close={close} />
      <SafetyStrip model={model} />
      <JourneyTrack state={model.state} />
      <ActionDeck model={model} compact />
      <ComparisonDeck model={model} />
      <ReceiptCard receipt={model.receipt} />
      <TraceLedger trace={model.trace} />
    </aside>
  );
}

function BenchVariant({
  model,
  close,
}: {
  readonly model: ReviewModel;
  readonly close: () => void;
}) {
  return (
    <section className="rust-flight rust-flight--bench" aria-label="Rust shadow comparison bench">
      <div className="rust-flight--bench__head">
        <PrototypeHeader name="B · Flight recorder bench" close={close} />
        <SafetyStrip model={model} />
      </div>
      <div className="rust-flight--bench__body">
        <div>
          <JourneyTrack state={model.state} />
          <ActionDeck model={model} compact />
        </div>
        <div>
          <ComparisonDeck model={model} />
          <ReceiptCard receipt={model.receipt} />
        </div>
        <TraceLedger trace={model.trace} />
      </div>
    </section>
  );
}

function LensVariant({
  model,
  close,
}: {
  readonly model: ReviewModel;
  readonly close: () => void;
}) {
  return (
    <section className="rust-flight rust-flight--lens" aria-label="Rust shadow cutover lens">
      <PrototypeHeader name="C · Cutover lens" close={close} />
      <div className="rust-flight--lens__headline">
        <span>ONE GUI</span>
        <strong>Two backend readings. One canonical writer.</strong>
        <p>
          Drive the candidate contract in Rust while the live Alfredo workstation remains on
          Python. Discard the shadow at any point.
        </p>
      </div>
      <SafetyStrip model={model} />
      <ComparisonDeck model={model} />
      <div className="rust-flight--lens__work">
        <ActionDeck model={model} />
        <div>
          <JourneyTrack state={model.state} />
          <ReceiptCard receipt={model.receipt} />
          <TraceLedger trace={model.trace} />
        </div>
      </div>
    </section>
  );
}

function PrototypeHeader({
  name,
  close,
}: {
  readonly name: string;
  readonly close: () => void;
}) {
  return (
    <header className="rust-flight-header">
      <div>
        <span className="rust-flight-kicker">PROTOTYPE · RUST SHADOW</span>
        <h2>{name}</h2>
      </div>
      <button type="button" onClick={close} aria-label="Hide Rust shadow review">
        Hide
      </button>
    </header>
  );
}

const variants = [
  ["A", "Shadow rail"],
  ["B", "Flight recorder bench"],
  ["C", "Cutover lens"],
] as const;
type Variant = (typeof variants)[number][0];

function currentVariant(): Variant {
  const value = new URLSearchParams(window.location.search).get("variant");
  return value === "B" || value === "C" ? value : "A";
}

function VariantSwitcher({
  variant,
  setVariant,
}: {
  readonly variant: Variant;
  readonly setVariant: (variant: Variant) => void;
}) {
  const index = variants.findIndex(([key]) => key === variant);
  const cycle = useCallback(
    (offset: number) => {
      setVariant(variants[(index + offset + variants.length) % variants.length][0]);
    },
    [index, setVariant],
  );
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      if (
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable)
      ) {
        return;
      }
      if (event.key === "ArrowLeft") cycle(-1);
      if (event.key === "ArrowRight") cycle(1);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [cycle]);
  return (
    <nav className="rust-flight-switcher" aria-label="Rust prototype variants">
      <button type="button" onClick={() => cycle(-1)} aria-label="Previous prototype variant">
        ←
      </button>
      <span>
        {variant} · {variants[index][1]}
      </span>
      <button type="button" onClick={() => cycle(1)} aria-label="Next prototype variant">
        →
      </button>
    </nav>
  );
}

export function RustOrchestratorGuiPrototype({
  children,
}: {
  readonly children: ReactElement;
}) {
  const model = useRustReviewModel();
  const [variant, setVariantState] = useState<Variant>(currentVariant);
  const [open, setOpen] = useState(true);
  const setVariant = useCallback((next: Variant) => {
    const url = new URL(window.location.href);
    url.searchParams.set("variant", next);
    window.history.replaceState({}, "", url);
    setVariantState(next);
    setOpen(true);
  }, []);
  const activeVariant = useMemo(() => {
    if (variant === "B") {
      return <BenchVariant model={model} close={() => setOpen(false)} />;
    }
    if (variant === "C") {
      return <LensVariant model={model} close={() => setOpen(false)} />;
    }
    return <RailVariant model={model} close={() => setOpen(false)} />;
  }, [model, variant]);

  return (
    <div className="rust-prototype-host" data-variant={variant}>
      <div className="rust-prototype-host__product">{children}</div>
      {open ? (
        activeVariant
      ) : (
        <button
          type="button"
          className="rust-flight-reopen"
          onClick={() => setOpen(true)}
        >
          Open Rust shadow
        </button>
      )}
      <VariantSwitcher variant={variant} setVariant={setVariant} />
    </div>
  );
}
