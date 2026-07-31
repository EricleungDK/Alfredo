"""PROTOTYPE ONLY: pure state model for attention-driven Local Agent supervision.

Question:
Can an Alfredo-owned supervision loop join canonical Local Agent state with
advisory runner observations, deliver actionable attention before advancing an
observer cursor, recover idempotently after faults, and stay silent/token-free
for healthy work without letting an observation mutate Mission state?

The reducer is deliberately free of I/O, clocks, UUID generation, and model
calls. The terminal shell in ``__main__.py`` supplies explicit actions.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Literal


DeliveryMode = Literal["atomic", "outbox-first"]
RecoveryPolicy = Literal["commander-visible", "automatic-after-proof"]

TERMINAL_STATUSES = {
    "completed",
    "evidence-ready",
    "reviewed",
    "failed",
    "cancelled",
}


@dataclass(frozen=True)
class RunnerBinding:
    pid: int
    start_identity: str
    process_group_identity: str
    operation_id: str


@dataclass(frozen=True)
class CanonicalSession:
    mission_id: str
    work_id: str
    session_id: str
    worktree_identity: str
    revision: int
    status: str
    runner: RunnerBinding | None
    recovery_count: int = 0
    result_receipt: str = ""


@dataclass(frozen=True)
class RunnerEvent:
    seq: int
    kind: str
    operation_id: str
    incident_key: str
    occurred_tick: int
    summary: str
    owner_signal: str
    process_group_signal: str
    worktree_signal: str
    operation_phase: str
    last_activity_tick: int
    persisted_result: str
    result_validation: str
    result_session_id: str
    result_operation_id: str
    result_worktree_identity: str
    diagnostic_output: str


@dataclass(frozen=True)
class RunnerWorld:
    tick: int
    operation_id: str
    owner_signal: str
    process_group_signal: str
    worktree_signal: str
    operation_phase: str
    last_activity_tick: int
    persisted_result: str
    result_validation: str
    result_session_id: str
    result_operation_id: str
    result_worktree_identity: str
    diagnostic_output: str
    condition_serial: int
    condition_key: str
    condition_started_tick: int | None
    events: tuple[RunnerEvent, ...]


@dataclass(frozen=True)
class CursorReceipt:
    seq: int
    disposition: str
    attention_id: str = ""


@dataclass(frozen=True)
class ObserverState:
    source_incarnation: str
    event_cursor: int
    cursor_receipts: tuple[CursorReceipt, ...]
    watcher_status: str
    next_fault: str
    unavailable_streak: int


@dataclass(frozen=True)
class AttentionRecord:
    attention_id: str
    incident_key: str
    kind: str
    severity: str
    canonical_revision_seen: int
    first_seen_tick: int
    detection_latency_ticks: int
    sources: tuple[str, ...]
    next_effect: str
    detail: str
    disposition: str = "open"
    resolution_receipt: str = ""


@dataclass(frozen=True)
class EffectIntent:
    intent_id: str
    attention_id: str
    kind: str
    correlation_id: str
    mission_id_expected: str
    session_id_expected: str
    canonical_revision_expected: int
    worktree_identity_expected: str
    runner_pid_expected: int
    runner_start_identity_expected: str
    process_group_identity_expected: str
    operation_id_expected: str
    result_digest_expected: str
    status: str = "pending"
    attempt_count: int = 0
    receipt_key: str = ""


@dataclass(frozen=True)
class EffectReceipt:
    receipt_key: str
    correlation_id: str
    effect: str
    outcome: str
    canonical_revision: int
    tick: int
    boundary: str = ""


@dataclass(frozen=True)
class BaselineState:
    status: str
    recovery_count: int
    startup_recovery_effects: int
    timeline_entries: int
    last_message: str


@dataclass(frozen=True)
class Metrics:
    event_attempts: int = 0
    reconciliation_sweeps: int = 0
    independent_probes: int = 0
    attention_writes: int = 0
    attention_dedupes: int = 0
    attention_resolutions: int = 0
    cursor_advances: int = 0
    notifications_dropped: int = 0
    restarts: int = 0
    domain_effects: int = 0
    duplicate_effects_prevented: int = 0
    model_turns: int = 0
    activity_journal_entries: int = 0
    healthy_user_emissions: int = 0
    observation_mission_mutations: int = 0


@dataclass(frozen=True)
class PrototypeConfig:
    delivery_mode: DeliveryMode = "atomic"
    recovery_policy: RecoveryPolicy = "commander-visible"
    tick_seconds: int = 15
    stale_after_ticks: int = 3
    unavailable_after_sweeps: int = 2


@dataclass(frozen=True)
class PrototypeState:
    canonical: CanonicalSession
    world: RunnerWorld
    observer: ObserverState
    attentions: tuple[AttentionRecord, ...]
    intents: tuple[EffectIntent, ...]
    receipts: tuple[EffectReceipt, ...]
    baseline: BaselineState
    metrics: Metrics
    config: PrototypeConfig
    last_intent_correlation: str
    last_message: str
    history: tuple[str, ...]


@dataclass(frozen=True)
class Action:
    kind: str
    value: str | int | None = None


@dataclass(frozen=True)
class Finding:
    incident_key: str
    kind: str
    severity: str
    detail: str
    next_effect: str
    started_tick: int
    result_digest: str = ""


@dataclass(frozen=True)
class Projection:
    headline: str
    detail: str
    next_action: str


INITIAL_RUNNER = RunnerBinding(
    pid=4127,
    start_identity="linux:4127:991004",
    process_group_identity="linux-pg:4127:991004",
    operation_id="op-ISS-53-1",
)


def initial_state(
    *,
    delivery_mode: DeliveryMode = "atomic",
    recovery_policy: RecoveryPolicy = "commander-visible",
    tick_seconds: int = 15,
    stale_after_ticks: int = 3,
    unavailable_after_sweeps: int = 2,
) -> PrototypeState:
    """Return one clean in-memory comparison run."""

    return PrototypeState(
        canonical=CanonicalSession(
            mission_id="modernize-alfredo",
            work_id="ISS-53",
            session_id="session-ISS-53-1",
            worktree_identity="managed:alfredo/ISS-53/session-1",
            revision=12,
            status="running",
            runner=INITIAL_RUNNER,
        ),
        world=RunnerWorld(
            tick=0,
            operation_id=INITIAL_RUNNER.operation_id,
            owner_signal="live-exact",
            process_group_signal="live-exact",
            worktree_signal="exact",
            operation_phase="generating",
            last_activity_tick=0,
            persisted_result="",
            result_validation="none",
            result_session_id="",
            result_operation_id="",
            result_worktree_identity="",
            diagnostic_output="",
            condition_serial=0,
            condition_key="",
            condition_started_tick=None,
            events=(),
        ),
        observer=ObserverState(
            source_incarnation="runner-events/boot-A",
            event_cursor=0,
            cursor_receipts=(),
            watcher_status="active",
            next_fault="none",
            unavailable_streak=0,
        ),
        attentions=(),
        intents=(),
        receipts=(),
        baseline=BaselineState(
            status="running",
            recovery_count=0,
            startup_recovery_effects=0,
            timeline_entries=0,
            last_message="Current Python waits for runner writes or Mission reload.",
        ),
        metrics=Metrics(),
        config=PrototypeConfig(
            delivery_mode=delivery_mode,
            recovery_policy=recovery_policy,
            tick_seconds=max(1, tick_seconds),
            stale_after_ticks=max(1, stale_after_ticks),
            unavailable_after_sweeps=max(1, unavailable_after_sweeps),
        ),
        last_intent_correlation="",
        last_message="Ready. Canonical state and advisory observations agree.",
        history=("Prototype reset.",),
    )


def _remember(state: PrototypeState, message: str) -> PrototypeState:
    history = (*state.history, message)[-6:]
    return replace(state, last_message=message, history=history)


def _condition(
    state: PrototypeState,
    name: str,
    *,
    tick_step: int = 1,
) -> tuple[RunnerWorld, str]:
    world = state.world
    serial = world.condition_serial + 1
    tick = world.tick + max(1, tick_step)
    key = (
        f"{state.canonical.mission_id}/{state.canonical.session_id}/"
        f"{world.operation_id}/{name}/g{serial}"
    )
    return (
        replace(
            world,
            tick=tick,
            condition_serial=serial,
            condition_key=key,
            condition_started_tick=tick,
        ),
        key,
    )


def _append_event(
    world: RunnerWorld,
    *,
    kind: str,
    incident_key: str,
    summary: str,
) -> RunnerWorld:
    event = RunnerEvent(
        seq=len(world.events) + 1,
        kind=kind,
        operation_id=world.operation_id,
        incident_key=incident_key,
        occurred_tick=world.tick,
        summary=summary,
        owner_signal=world.owner_signal,
        process_group_signal=world.process_group_signal,
        worktree_signal=world.worktree_signal,
        operation_phase=world.operation_phase,
        last_activity_tick=world.last_activity_tick,
        persisted_result=world.persisted_result,
        result_validation=world.result_validation,
        result_session_id=world.result_session_id,
        result_operation_id=world.result_operation_id,
        result_worktree_identity=world.result_worktree_identity,
        diagnostic_output=world.diagnostic_output,
    )
    return replace(world, events=(*world.events, event))


def _inject_healthy(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: the canonical session is no longer Running.")
    tick = state.world.tick + 1
    state = replace(
        state,
        world=replace(
            state.world,
            tick=tick,
            owner_signal="live-exact",
            process_group_signal="live-exact",
            worktree_signal="exact",
            operation_phase="generating",
            last_activity_tick=tick,
            persisted_result="",
            result_validation="none",
            result_session_id="",
            result_operation_id="",
            result_worktree_identity="",
            diagnostic_output="",
            condition_key="",
            condition_started_tick=None,
        ),
    )
    return _sweep(state, reason="healthy tick")


def _inject_normal_completion(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: normal completion requires Running.")
    world, key = _condition(state, "normal-completion")
    result = "sha256:validated-result-53"
    world = replace(
        world,
        owner_signal="absent",
        process_group_signal="absent",
        operation_phase="finished",
        persisted_result=result,
        result_validation="exact-valid",
        result_session_id=state.canonical.session_id,
        result_operation_id=world.operation_id,
        result_worktree_identity=state.canonical.worktree_identity,
        last_activity_tick=world.tick,
    )
    world = _append_event(
        world,
        kind="completion",
        incident_key=key,
        summary="Runner committed its result and canonical terminal receipt.",
    )
    canonical = replace(
        state.canonical,
        revision=state.canonical.revision + 1,
        status="evidence-ready",
        runner=None,
        result_receipt="result-receipt-normal",
    )
    baseline = replace(
        state.baseline,
        status="evidence-ready",
        timeline_entries=state.baseline.timeline_entries + 1,
        last_message="Runner wrote Evidence Ready before exit.",
    )
    state = _resolve_cleared_attentions(
        replace(state, canonical=canonical, world=world, baseline=baseline)
    )
    return _remember(
        state,
        "Authoritative runner completion committed; the later event is advisory.",
    )


def _inject_missed_completion(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: missed completion requires Running.")
    world, key = _condition(state, "missed-completion")
    world = replace(
        world,
        owner_signal="absent",
        process_group_signal="absent",
        operation_phase="finished",
        persisted_result="sha256:validated-result-53",
        result_validation="exact-valid",
        result_session_id=state.canonical.session_id,
        result_operation_id=world.operation_id,
        result_worktree_identity=state.canonical.worktree_identity,
        last_activity_tick=world.tick,
    )
    world = _append_event(
        world,
        kind="completion",
        incident_key=key,
        summary="Exact result exists; canonical completion write was missed.",
    )
    metrics = replace(
        state.metrics,
        notifications_dropped=state.metrics.notifications_dropped + 1,
    )
    return _remember(
        replace(state, world=world, metrics=metrics),
        "Completion callback dropped. Canonical state is still Running.",
    )


def _inject_invalid_result(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: invalid result requires Running.")
    world, key = _condition(state, "invalid-result")
    world = replace(
        world,
        owner_signal="absent",
        process_group_signal="absent",
        operation_phase="finished-with-invalid-result",
        persisted_result="sha256:untrusted-result-53",
        result_validation="hash-mismatch",
        result_session_id=state.canonical.session_id,
        result_operation_id=world.operation_id,
        result_worktree_identity=state.canonical.worktree_identity,
    )
    world = _append_event(
        world,
        kind="completion",
        incident_key=key,
        summary="A persisted result exists, but its content hash does not validate.",
    )
    return _remember(
        replace(state, world=world),
        "Invalid persisted result injected; it blocks both completion and rerun.",
    )


def _inject_dead_owner(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: dead-owner injection requires Running.")
    world, key = _condition(state, "dead-owner")
    world = replace(
        world,
        owner_signal="absent",
        process_group_signal="absent",
        operation_phase="owner-lost",
        persisted_result="",
        result_validation="none",
        result_session_id="",
        result_operation_id="",
        result_worktree_identity="",
    )
    world = _append_event(
        world,
        kind="owner-exit",
        incident_key=key,
        summary="Exact owner and process group are absent; no result exists.",
    )
    return _remember(
        replace(state, world=world),
        "Dead owner injected. Process-group absence and result absence are observable.",
    )


def _inject_pid_reuse(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: PID-reuse injection requires Running.")
    world, key = _condition(state, "pid-reuse")
    world = replace(
        world,
        owner_signal="pid-reused",
        process_group_signal="unknown",
        operation_phase="ownership-ambiguous",
        persisted_result="",
        result_validation="none",
        result_session_id="",
        result_operation_id="",
        result_worktree_identity="",
    )
    world = _append_event(
        world,
        kind="identity-mismatch",
        incident_key=key,
        summary="PID exists with a different Linux start identity; group probe is unknown.",
    )
    return _remember(
        replace(state, world=world),
        "PID reuse injected. The replacement process is not the recorded owner.",
    )


def _inject_stale_activity(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: stale activity requires Running.")
    world, _ = _condition(
        state,
        "stale-activity",
        tick_step=state.config.stale_after_ticks,
    )
    world = replace(
        world,
        owner_signal="live-exact",
        process_group_signal="live-exact",
        operation_phase="generating",
        persisted_result="",
    )
    return _remember(
        replace(state, world=world),
        f"Activity aged to the configured {state.config.stale_after_ticks}-tick threshold.",
    )


def _inject_contradictory_output(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: contradictory output requires Running.")
    world, key = _condition(state, "contradictory-output")
    world = replace(
        world,
        owner_signal="live-exact",
        process_group_signal="live-exact",
        diagnostic_output="DONE — all work complete",
    )
    world = _append_event(
        world,
        kind="diagnostic-output",
        incident_key=key,
        summary="Bounded terminal tail says DONE while canonical state says Running.",
    )
    return _remember(
        replace(state, world=world),
        "Contradictory terminal prose injected; it remains advisory.",
    )


def _inject_unavailable_observation(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: unavailable probe requires Running.")
    world, _ = _condition(state, "observation-unavailable")
    world = replace(
        world,
        owner_signal="unavailable",
        process_group_signal="unknown",
        operation_phase="observation-unavailable",
    )
    return _remember(
        replace(state, world=world),
        "Runner and process-group probes are unavailable; no success is inferred.",
    )


def _inject_terminal_owner_live(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: terminal/live injection requires Running.")
    world, key = _condition(state, "terminal-owner-live")
    world = replace(
        world,
        owner_signal="live-exact",
        process_group_signal="live-exact",
        operation_phase="still-running-after-terminal",
    )
    world = _append_event(
        world,
        kind="canonical-terminal",
        incident_key=key,
        summary="Canonical terminal receipt exists while the exact owner remains live.",
    )
    canonical = replace(
        state.canonical,
        revision=state.canonical.revision + 1,
        status="completed",
    )
    baseline = replace(
        state.baseline,
        status="completed",
        timeline_entries=state.baseline.timeline_entries + 1,
        last_message="Terminal label written; current projection has no quiescence ledger.",
    )
    state = _resolve_cleared_attentions(
        replace(state, canonical=canonical, world=world, baseline=baseline)
    )
    return _remember(
        state,
        "Canonical completion remains true, but Runner Quiescence is not proven.",
    )


def _inject_inference_wait(state: PrototypeState) -> PrototypeState:
    if state.canonical.status != "running":
        return _remember(state, "Reset first: inference wait requires Running.")
    tick = state.world.tick + 1
    world = replace(
        state.world,
        tick=tick,
        owner_signal="live-exact",
        process_group_signal="live-exact",
        operation_phase="waiting-local-inference-lease",
        last_activity_tick=tick,
        diagnostic_output="",
        condition_key="",
        condition_started_tick=None,
    )
    return _remember(
        replace(state, world=world),
        "Local Inference Lease wait injected; it is not a runner-liveness failure.",
    )


def _result_is_exact(
    state: PrototypeState,
    *,
    intent: EffectIntent | None = None,
    event: RunnerEvent | None = None,
) -> bool:
    world = state.world
    canonical = state.canonical
    persisted_result = (
        event.persisted_result if event is not None else world.persisted_result
    )
    result_validation = (
        event.result_validation if event is not None else world.result_validation
    )
    result_session_id = (
        event.result_session_id if event is not None else world.result_session_id
    )
    result_operation_id = (
        event.result_operation_id
        if event is not None
        else world.result_operation_id
    )
    result_worktree_identity = (
        event.result_worktree_identity
        if event is not None
        else world.result_worktree_identity
    )
    expected_operation = (
        intent.operation_id_expected
        if intent is not None
        else canonical.runner.operation_id
        if canonical.runner is not None
        else world.operation_id
    )
    expected_worktree = (
        intent.worktree_identity_expected
        if intent is not None
        else canonical.worktree_identity
    )
    expected_session = (
        intent.session_id_expected
        if intent is not None
        else canonical.session_id
    )
    expected_digest = (
        intent.result_digest_expected
        if intent is not None
        else persisted_result
    )
    return (
        bool(persisted_result)
        and result_validation == "exact-valid"
        and result_session_id == expected_session
        and result_operation_id == expected_operation
        and result_worktree_identity == expected_worktree
        and persisted_result == expected_digest
    )


def _finding(
    state: PrototypeState,
    *,
    event: RunnerEvent | None = None,
) -> Finding | None:
    canonical = state.canonical
    world = state.world
    owner_signal = event.owner_signal if event is not None else world.owner_signal
    process_group_signal = (
        event.process_group_signal
        if event is not None
        else world.process_group_signal
    )
    worktree_signal = (
        event.worktree_signal if event is not None else world.worktree_signal
    )
    operation_phase = (
        event.operation_phase if event is not None else world.operation_phase
    )
    last_activity_tick = (
        event.last_activity_tick if event is not None else world.last_activity_tick
    )
    persisted_result = (
        event.persisted_result if event is not None else world.persisted_result
    )
    diagnostic_output = (
        event.diagnostic_output if event is not None else world.diagnostic_output
    )
    observation_tick = event.occurred_tick if event is not None else world.tick
    incident_key = (
        event.incident_key
        if event is not None
        else world.condition_key
        or f"{canonical.session_id}/{world.operation_id}/sweep"
    )
    started_tick = (
        event.occurred_tick
        if event is not None
        else world.condition_started_tick
        if world.condition_started_tick is not None
        else world.tick
    )

    if (
        event is not None
        and canonical.runner is not None
        and event.operation_id != canonical.runner.operation_id
    ):
        return Finding(
            incident_key=incident_key,
            kind="stale-operation-observation",
            severity="diagnostic",
            detail="Observation belongs to a different runner operation.",
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    if canonical.status in TERMINAL_STATUSES:
        if (
            owner_signal == "live-exact"
            or process_group_signal == "live-exact"
        ):
            return Finding(
                incident_key=incident_key,
                kind="terminal-owner-live",
                severity="high",
                detail=(
                    "Canonical terminal state wins, but the exact runner/process "
                    "group is still live; retirement is blocked."
                ),
                next_effect="wait-for-quiescence",
                started_tick=started_tick,
            )
        if (
            owner_signal == "unavailable"
            or process_group_signal == "unknown"
        ):
            return Finding(
                incident_key=incident_key,
                kind="quiescence-unknown",
                severity="high",
                detail="Terminal state is authoritative; Runner Quiescence is unknown.",
                next_effect="inspect-only",
                started_tick=started_tick,
            )
        return None

    if canonical.status != "running":
        return None

    if worktree_signal != "exact":
        return Finding(
            incident_key=incident_key,
            kind="worktree-identity-unknown",
            severity="high",
            detail="Observed worktree identity does not match the canonical session.",
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    if owner_signal == "pid-reused":
        return Finding(
            incident_key=incident_key,
            kind="ownership-ambiguous",
            severity="high",
            detail=(
                "The PID has a different start identity and process-group "
                "absence is not proven; duplicate execution is forbidden."
            ),
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    if owner_signal == "unavailable":
        if state.observer.unavailable_streak < state.config.unavailable_after_sweeps:
            return None
        return Finding(
            incident_key=incident_key,
            kind="liveness-unavailable",
            severity="medium",
            detail=(
                "Independent runner evidence is unavailable after the bounded "
                "grace threshold; no success or recovery is inferred."
            ),
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    if owner_signal == "absent":
        if process_group_signal != "absent":
            return Finding(
                incident_key=incident_key,
                kind="ownership-ambiguous",
                severity="high",
                detail="Owner is absent, but child/process-group quiescence is unknown.",
                next_effect="inspect-only",
                started_tick=started_tick,
            )
        if persisted_result:
            if not _result_is_exact(state, event=event):
                return Finding(
                    incident_key=incident_key,
                    kind="result-validation-failed",
                    severity="high",
                    detail=(
                        "Persisted bytes exist, but their session, operation, "
                        "worktree, or content validation failed; completion and "
                        "rerun are both blocked."
                    ),
                    next_effect="inspect-only",
                    started_tick=started_tick,
                )
            return Finding(
                incident_key=incident_key,
                kind="result-unreconciled",
                severity="high",
                detail=(
                    "An exact persisted result exists while canonical state is "
                    "Running; validate it before any rerun."
                ),
                next_effect="reconcile-result",
                started_tick=started_tick,
                result_digest=persisted_result,
            )
        return Finding(
            incident_key=incident_key,
            kind="dead-owner",
            severity="high",
            detail=(
                "Exact owner and process group are absent and no result exists; "
                "same-session recovery may be attempted under the lifecycle lock."
            ),
            next_effect="recover-same-session",
            started_tick=started_tick,
        )

    if persisted_result:
        if not _result_is_exact(state, event=event):
            return Finding(
                incident_key=incident_key,
                kind="result-validation-failed",
                severity="high",
                detail="A live-owner result exists but exact validation failed.",
                next_effect="inspect-only",
                started_tick=started_tick,
            )
        return Finding(
            incident_key=incident_key,
            kind="result-with-live-owner",
            severity="medium",
            detail="A result exists, but the exact owner is still live.",
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    if diagnostic_output:
        return Finding(
            incident_key=incident_key,
            kind="diagnostic-conflict",
            severity="diagnostic",
            detail="Terminal/provider prose conflicts with canonical Running state.",
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    activity_age = observation_tick - last_activity_tick
    if (
        operation_phase != "waiting-local-inference-lease"
        and activity_age >= state.config.stale_after_ticks
    ):
        return Finding(
            incident_key=incident_key,
            kind="observation-stale",
            severity="medium",
            detail=(
                f"Exact owner is live, but activity is {activity_age} ticks old; "
                "staleness cannot authorize recovery."
            ),
            next_effect="inspect-only",
            started_tick=started_tick,
        )

    return None


def _upsert_attention(
    state: PrototypeState,
    finding: Finding,
    *,
    source: str,
) -> tuple[PrototypeState, str]:
    for index, attention in enumerate(state.attentions):
        same_incident = attention.incident_key == finding.incident_key
        same_open_condition = (
            attention.disposition == "open"
            and attention.canonical_revision_seen == state.canonical.revision
            and (
                attention.kind == finding.kind
                or (
                    attention.next_effect
                    in {"reconcile-result", "recover-same-session"}
                    and attention.next_effect == finding.next_effect
                )
            )
        )
        if not same_incident and not same_open_condition:
            continue
        sources = attention.sources
        if source not in sources:
            sources = (*sources, source)
        updated = replace(attention, sources=sources)
        attentions = list(state.attentions)
        attentions[index] = updated
        metrics = replace(
            state.metrics,
            attention_dedupes=state.metrics.attention_dedupes + 1,
        )
        return replace(state, attentions=tuple(attentions), metrics=metrics), attention.attention_id

    attention_id = f"ATT-{len(state.attentions) + 1:03d}"
    attention = AttentionRecord(
        attention_id=attention_id,
        incident_key=finding.incident_key,
        kind=finding.kind,
        severity=finding.severity,
        canonical_revision_seen=state.canonical.revision,
        first_seen_tick=state.world.tick,
        detection_latency_ticks=max(0, state.world.tick - finding.started_tick),
        sources=(source,),
        next_effect=finding.next_effect,
        detail=finding.detail,
    )
    intents = state.intents
    if finding.next_effect in {"reconcile-result", "recover-same-session"}:
        runner = state.canonical.runner
        if runner is None:
            raise ValueError("An automatic supervision intent requires a runner binding.")
        correlation = (
            f"supervise:{state.canonical.mission_id}:{state.canonical.session_id}:"
            f"{runner.operation_id}:{finding.next_effect}:"
            f"r{state.canonical.revision}:{finding.incident_key}"
        )
        intents = (
            *intents,
            EffectIntent(
                intent_id=f"INT-{len(intents) + 1:03d}",
                attention_id=attention_id,
                kind=finding.next_effect,
                correlation_id=correlation,
                mission_id_expected=state.canonical.mission_id,
                session_id_expected=state.canonical.session_id,
                canonical_revision_expected=state.canonical.revision,
                worktree_identity_expected=state.canonical.worktree_identity,
                runner_pid_expected=runner.pid,
                runner_start_identity_expected=runner.start_identity,
                process_group_identity_expected=runner.process_group_identity,
                operation_id_expected=runner.operation_id,
                result_digest_expected=(
                    finding.result_digest
                    if finding.next_effect == "reconcile-result"
                    else ""
                ),
            ),
        )
    metrics = replace(
        state.metrics,
        attention_writes=state.metrics.attention_writes + 1,
    )
    return (
        replace(
            state,
            attentions=(*state.attentions, attention),
            intents=intents,
            metrics=metrics,
        ),
        attention_id,
    )


def _attention_condition_is_active(
    state: PrototypeState,
    attention: AttentionRecord,
) -> bool:
    canonical = state.canonical
    world = state.world
    kind = attention.kind
    if attention.next_effect == "reconcile-result":
        return (
            canonical.status == "running"
            and canonical.revision == attention.canonical_revision_seen
            and world.owner_signal == "absent"
            and world.process_group_signal == "absent"
            and _result_is_exact(state)
        )
    if attention.next_effect == "recover-same-session":
        return (
            canonical.status == "running"
            and canonical.revision == attention.canonical_revision_seen
            and world.owner_signal == "absent"
            and world.process_group_signal == "absent"
            and world.worktree_signal == "exact"
            and not world.persisted_result
        )
    if kind == "observation-stale":
        return (
            canonical.status == "running"
            and world.owner_signal == "live-exact"
            and world.operation_phase != "waiting-local-inference-lease"
            and world.tick - world.last_activity_tick
            >= state.config.stale_after_ticks
        )
    if kind == "diagnostic-conflict":
        return canonical.status == "running" and bool(world.diagnostic_output)
    if kind == "liveness-unavailable":
        return (
            canonical.status == "running"
            and world.owner_signal == "unavailable"
        )
    if kind == "ownership-ambiguous":
        return canonical.status == "running" and (
            world.owner_signal == "pid-reused"
            or (
                world.owner_signal == "absent"
                and world.process_group_signal != "absent"
            )
        )
    if kind == "worktree-identity-unknown":
        return canonical.status == "running" and world.worktree_signal != "exact"
    if kind == "result-with-live-owner":
        return (
            canonical.status == "running"
            and bool(world.persisted_result)
            and world.owner_signal == "live-exact"
        )
    if kind == "result-validation-failed":
        return (
            canonical.status == "running"
            and bool(world.persisted_result)
            and not _result_is_exact(state)
        )
    if kind == "terminal-owner-live":
        return canonical.status in TERMINAL_STATUSES and (
            world.owner_signal == "live-exact"
            or world.process_group_signal == "live-exact"
        )
    if kind == "quiescence-unknown":
        return canonical.status in TERMINAL_STATUSES and (
            world.owner_signal == "unavailable"
            or world.process_group_signal == "unknown"
        )
    return True


def _resolve_cleared_attentions(state: PrototypeState) -> PrototypeState:
    attentions = list(state.attentions)
    intents = list(state.intents)
    receipts = list(state.receipts)
    resolved_count = 0
    for index, attention in enumerate(attentions):
        if attention.disposition != "open":
            continue
        if _attention_condition_is_active(state, attention):
            continue
        receipt_key = f"RCPT-{len(receipts) + 1:03d}"
        disposition = (
            "superseded"
            if state.canonical.revision > attention.canonical_revision_seen
            else "resolved"
        )
        receipt = EffectReceipt(
            receipt_key=receipt_key,
            correlation_id=(
                f"resolve:{attention.attention_id}:"
                f"r{state.canonical.revision}:t{state.world.tick}"
            ),
            effect="resolve-attention",
            outcome=(
                f"{attention.kind} condition cleared under canonical "
                f"revision {state.canonical.revision}."
            ),
            canonical_revision=state.canonical.revision,
            tick=state.world.tick,
        )
        receipts.append(receipt)
        attentions[index] = replace(
            attention,
            disposition=disposition,
            resolution_receipt=receipt_key,
        )
        for intent_index, intent in enumerate(intents):
            if (
                intent.attention_id == attention.attention_id
                and intent.status in {"pending", "in-flight"}
            ):
                intents[intent_index] = replace(
                    intent,
                    status="rejected",
                    receipt_key=receipt_key,
                )
        resolved_count += 1
    if not resolved_count:
        return state
    return replace(
        state,
        attentions=tuple(attentions),
        intents=tuple(intents),
        receipts=tuple(receipts),
        metrics=replace(
            state.metrics,
            attention_resolutions=(
                state.metrics.attention_resolutions + resolved_count
            ),
        ),
    )


def _advance_cursor(
    state: PrototypeState,
    event: RunnerEvent,
    *,
    disposition: str,
    attention_id: str = "",
) -> PrototypeState:
    expected = state.observer.event_cursor + 1
    if event.seq != expected:
        return _remember(
            state,
            f"Cursor gap: expected event {expected}, received {event.seq}; no advance.",
        )
    observer = replace(
        state.observer,
        event_cursor=event.seq,
        cursor_receipts=(
            *state.observer.cursor_receipts,
            CursorReceipt(
                seq=event.seq,
                disposition=disposition,
                attention_id=attention_id,
            ),
        ),
        watcher_status="active",
    )
    metrics = replace(
        state.metrics,
        cursor_advances=state.metrics.cursor_advances + 1,
    )
    return replace(state, observer=observer, metrics=metrics)


def _process_next_event(state: PrototypeState) -> PrototypeState:
    if state.observer.watcher_status == "crashed":
        return _remember(state, "Watcher is crashed; restart it before reading events.")
    if state.observer.event_cursor >= len(state.world.events):
        return _remember(state, "No runner event is waiting behind the cursor.")

    event = state.world.events[state.observer.event_cursor]
    state = replace(
        state,
        metrics=replace(
            state.metrics,
            event_attempts=state.metrics.event_attempts + 1,
        ),
    )
    finding = _finding(state, event=event)
    if finding is None:
        state = _resolve_cleared_attentions(state)
        state = _advance_cursor(state, event, disposition="benign")
        return _remember(
            state,
            f"Event {event.seq} was benign under canonical precedence; cursor advanced.",
        )

    fault = state.observer.next_fault
    if fault != "none" and state.config.delivery_mode == "atomic":
        watcher_status = "crashed" if fault == "crash-after-attention" else "degraded"
        observer = replace(
            state.observer,
            watcher_status=watcher_status,
            next_fault="none",
        )
        return _remember(
            replace(state, observer=observer),
            (
                f"Atomic ledger commit aborted at {fault}: neither attention nor "
                f"cursor for event {event.seq} became durable."
            ),
        )

    state, attention_id = _upsert_attention(
        state,
        finding,
        source=f"event:{state.observer.source_incarnation}:{event.seq}",
    )
    state = _resolve_cleared_attentions(state)
    if fault != "none":
        watcher_status = "crashed" if fault == "crash-after-attention" else "degraded"
        observer = replace(
            state.observer,
            watcher_status=watcher_status,
            next_fault="none",
        )
        return _remember(
            replace(state, observer=observer),
            (
                f"Attention {attention_id} is durable, then {fault} prevented "
                f"cursor {event.seq}; replay must deduplicate."
            ),
        )

    state = _advance_cursor(
        state,
        event,
        disposition="actionable",
        attention_id=attention_id,
    )
    state = _remember(
        state,
        f"Delivered {attention_id} and advanced event cursor to {event.seq}.",
    )
    return _maybe_apply_automatic_recovery(state)


def _sweep(state: PrototypeState, *, reason: str = "manual") -> PrototypeState:
    observer = state.observer
    if state.world.owner_signal == "unavailable":
        observer = replace(
            observer,
            unavailable_streak=observer.unavailable_streak + 1,
        )
    else:
        observer = replace(observer, unavailable_streak=0)
    metrics = replace(
        state.metrics,
        reconciliation_sweeps=state.metrics.reconciliation_sweeps + 1,
        independent_probes=state.metrics.independent_probes + 3,
    )
    state = replace(state, observer=observer, metrics=metrics)
    finding = _finding(state)
    if finding is None:
        state = _resolve_cleared_attentions(state)
        if state.world.owner_signal == "unavailable":
            return _remember(
                state,
                (
                    f"NoChange ({reason}): unavailable grace "
                    f"{state.observer.unavailable_streak}/"
                    f"{state.config.unavailable_after_sweeps}; 0 model turns."
                ),
            )
        return _remember(
            state,
            f"NoChange ({reason}): bounded probes only; 0 model turns and 0 emissions.",
        )
    state, attention_id = _upsert_attention(
        state,
        finding,
        source=f"sweep:{state.metrics.reconciliation_sweeps}",
    )
    state = _resolve_cleared_attentions(state)
    state = _remember(
        state,
        f"Reconciliation sweep delivered or merged {attention_id}: {finding.kind}.",
    )
    return _maybe_apply_automatic_recovery(state)


def _baseline_restart(state: PrototypeState) -> BaselineState:
    baseline = state.baseline
    if baseline.status != "running":
        return replace(
            baseline,
            last_message="Mission reload made no abandoned-runner lifecycle change.",
        )
    if state.world.owner_signal == "live-exact":
        return replace(
            baseline,
            last_message="Mission reload still sees the exact Python owner as live.",
        )
    recovery_count = baseline.recovery_count + 1
    status = "queued" if recovery_count <= 3 else "failed"
    result_note = (
        " Persisted result is not checked before this requeue."
        if state.world.persisted_result
        else ""
    )
    return replace(
        baseline,
        status=status,
        recovery_count=recovery_count,
        startup_recovery_effects=baseline.startup_recovery_effects + 1,
        timeline_entries=baseline.timeline_entries + 1,
        last_message=(
            f"Mission load changed Running → {status.title()} from PID/start "
            f"liveness alone.{result_note}"
        ),
    )


def _restart(state: PrototypeState) -> PrototypeState:
    tick = state.world.tick + 1
    state = replace(
        state,
        world=replace(state.world, tick=tick),
        observer=replace(
            state.observer,
            watcher_status="active",
            next_fault="none",
        ),
        baseline=_baseline_restart(state),
        metrics=replace(state.metrics, restarts=state.metrics.restarts + 1),
    )
    state = _remember(
        state,
        "Restart restored durable ledger state, retained cursors, and scheduled reconciliation.",
    )
    if state.observer.event_cursor < len(state.world.events):
        state = _process_next_event(state)
    return _sweep(state, reason="startup")


def _replace_intent(
    intents: tuple[EffectIntent, ...],
    updated: EffectIntent,
) -> tuple[EffectIntent, ...]:
    return tuple(updated if item.intent_id == updated.intent_id else item for item in intents)


def _resolve_attention(
    attentions: tuple[AttentionRecord, ...],
    *,
    attention_id: str,
    receipt_key: str,
    new_revision: int,
) -> tuple[AttentionRecord, ...]:
    resolved: list[AttentionRecord] = []
    for attention in attentions:
        if attention.attention_id == attention_id:
            resolved.append(
                replace(
                    attention,
                    disposition="resolved",
                    resolution_receipt=receipt_key,
                )
            )
        elif (
            attention.disposition == "open"
            and attention.canonical_revision_seen < new_revision
        ):
            resolved.append(
                replace(
                    attention,
                    disposition="superseded",
                    resolution_receipt=receipt_key,
                )
            )
        else:
            resolved.append(attention)
    return tuple(resolved)


def _apply_next_intent(
    state: PrototypeState,
    *,
    only_kind: str | None = None,
) -> PrototypeState:
    pending = next(
        (
            intent
            for intent in state.intents
            if intent.status in {"pending", "in-flight"}
            and (only_kind is None or intent.kind == only_kind)
        ),
        None,
    )
    if pending is None:
        if only_kind is not None:
            return state
        if state.last_intent_correlation and any(
            receipt.correlation_id == state.last_intent_correlation
            for receipt in state.receipts
        ):
            metrics = replace(
                state.metrics,
                duplicate_effects_prevented=(
                    state.metrics.duplicate_effects_prevented + 1
                ),
            )
            return _remember(
                replace(state, metrics=metrics),
                (
                    "Duplicate effect request replayed its exact Orchestrator "
                    "receipt; no second domain mutation occurred."
                ),
            )
        return _remember(state, "No automatic typed intent is ready to apply.")

    intent = replace(
        pending,
        status="in-flight",
        attempt_count=pending.attempt_count + 1,
    )
    state = replace(state, intents=_replace_intent(state.intents, intent))
    canonical = state.canonical
    world = state.world
    runner = canonical.runner
    valid_boundary = (
        canonical.mission_id == intent.mission_id_expected
        and canonical.session_id == intent.session_id_expected
        and canonical.revision == intent.canonical_revision_expected
        and canonical.status == "running"
        and canonical.worktree_identity == intent.worktree_identity_expected
        and world.worktree_signal == "exact"
        and runner is not None
        and runner.pid == intent.runner_pid_expected
        and runner.start_identity == intent.runner_start_identity_expected
        and (
            runner.process_group_identity
            == intent.process_group_identity_expected
        )
        and runner.operation_id == intent.operation_id_expected
        and world.operation_id == intent.operation_id_expected
        and world.owner_signal == "absent"
        and world.process_group_signal == "absent"
    )
    if intent.kind == "reconcile-result":
        valid_boundary = valid_boundary and _result_is_exact(state, intent=intent)
    elif intent.kind == "recover-same-session":
        valid_boundary = valid_boundary and not world.persisted_result
    else:
        valid_boundary = False

    if not valid_boundary:
        receipt_key = f"RCPT-{len(state.receipts) + 1:03d}"
        rejected = replace(
            intent,
            status="rejected",
            receipt_key=receipt_key,
        )
        receipt = EffectReceipt(
            receipt_key=receipt_key,
            correlation_id=intent.correlation_id,
            effect=intent.kind,
            outcome=(
                "Rejected: exact revision, runner/operation/worktree identity, "
                "quiescence, and result boundary did not all hold."
            ),
            canonical_revision=canonical.revision,
            tick=world.tick,
            boundary=(
                f"{intent.mission_id_expected}/{intent.session_id_expected} · "
                f"{intent.worktree_identity_expected} · "
                f"op={intent.operation_id_expected} · "
                f"pid={intent.runner_pid_expected}@"
                f"{intent.runner_start_identity_expected} · "
                f"group={intent.process_group_identity_expected} · "
                f"expected-r{intent.canonical_revision_expected}"
            ),
        )
        return _remember(
            replace(
                state,
                attentions=_resolve_attention(
                    state.attentions,
                    attention_id=intent.attention_id,
                    receipt_key=receipt_key,
                    new_revision=canonical.revision,
                ),
                intents=_replace_intent(state.intents, rejected),
                receipts=(*state.receipts, receipt),
                metrics=replace(
                    state.metrics,
                    attention_resolutions=(
                        state.metrics.attention_resolutions + 1
                    ),
                ),
                last_intent_correlation=intent.correlation_id,
            ),
            (
                f"{intent.kind} was rejected: exact revision, identity, "
                "quiescence, and result boundary did not all hold."
            ),
        )

    next_revision = canonical.revision + 1
    if intent.kind == "reconcile-result":
        canonical = replace(
            canonical,
            revision=next_revision,
            status="evidence-ready",
            runner=None,
            result_receipt=f"result:{world.persisted_result}",
        )
        outcome = "Exact persisted result reconciled; Evidence Ready."
    else:
        canonical = replace(
            canonical,
            revision=next_revision,
            status="queued",
            runner=None,
            recovery_count=canonical.recovery_count + 1,
        )
        outcome = "Same session/worktree queued once for infrastructure recovery."

    receipt_key = f"RCPT-{len(state.receipts) + 1:03d}"
    receipt = EffectReceipt(
        receipt_key=receipt_key,
        correlation_id=intent.correlation_id,
        effect=intent.kind,
        outcome=outcome,
        canonical_revision=next_revision,
        tick=world.tick,
        boundary=(
            f"{intent.mission_id_expected}/{intent.session_id_expected} · "
            f"{intent.worktree_identity_expected} · "
            f"op={intent.operation_id_expected} · "
            f"pid={intent.runner_pid_expected}@"
            f"{intent.runner_start_identity_expected} · "
            f"group={intent.process_group_identity_expected} · "
            f"expected-r{intent.canonical_revision_expected}"
            + (
                f" · result={intent.result_digest_expected}"
                if intent.result_digest_expected
                else " · result=absent"
            )
        ),
    )
    applied = replace(
        intent,
        status="applied",
        receipt_key=receipt_key,
    )
    metrics = replace(
        state.metrics,
        domain_effects=state.metrics.domain_effects + 1,
    )
    return _remember(
        replace(
            state,
            canonical=canonical,
            attentions=_resolve_attention(
                state.attentions,
                attention_id=intent.attention_id,
                receipt_key=receipt_key,
                new_revision=next_revision,
            ),
            intents=_replace_intent(state.intents, applied),
            receipts=(*state.receipts, receipt),
            metrics=metrics,
            last_intent_correlation=intent.correlation_id,
        ),
        f"Applied {intent.kind} through {receipt_key}; canonical revision is {next_revision}.",
    )


def _maybe_apply_automatic_recovery(state: PrototypeState) -> PrototypeState:
    if state.config.recovery_policy != "automatic-after-proof":
        return state
    return _apply_next_intent(state, only_kind="recover-same-session")


def transition(state: PrototypeState, action: Action) -> PrototypeState:
    """Purely reduce one explicit prototype action."""

    kind = action.kind
    if kind == "reset":
        return initial_state(
            delivery_mode=state.config.delivery_mode,
            recovery_policy=state.config.recovery_policy,
            tick_seconds=state.config.tick_seconds,
            stale_after_ticks=state.config.stale_after_ticks,
            unavailable_after_sweeps=state.config.unavailable_after_sweeps,
        )
    if kind == "healthy":
        return _inject_healthy(state)
    if kind == "normal-completion":
        return _inject_normal_completion(state)
    if kind == "missed-completion":
        return _inject_missed_completion(state)
    if kind == "invalid-result":
        return _inject_invalid_result(state)
    if kind == "dead-owner":
        return _inject_dead_owner(state)
    if kind == "pid-reuse":
        return _inject_pid_reuse(state)
    if kind == "stale-activity":
        return _inject_stale_activity(state)
    if kind == "contradictory-output":
        return _inject_contradictory_output(state)
    if kind == "unavailable":
        return _inject_unavailable_observation(state)
    if kind == "terminal-owner-live":
        return _inject_terminal_owner_live(state)
    if kind == "inference-wait":
        return _inject_inference_wait(state)
    if kind == "watch":
        return _process_next_event(state)
    if kind == "sweep":
        return _sweep(
            replace(state, world=replace(state.world, tick=state.world.tick + 1)),
            reason="manual",
        )
    if kind == "arm-crash":
        return _remember(
            replace(
                state,
                observer=replace(
                    state.observer,
                    next_fault="crash-after-attention",
                ),
            ),
            "Armed a watcher crash at the attention/cursor commit cut.",
        )
    if kind == "arm-cursor-failure":
        return _remember(
            replace(
                state,
                observer=replace(
                    state.observer,
                    next_fault="cursor-write-failure",
                ),
            ),
            "Armed a cursor-write failure for the next actionable event.",
        )
    if kind == "restart":
        return _restart(state)
    if kind == "apply":
        return _apply_next_intent(state)
    if kind == "set-delivery-mode":
        mode = str(action.value)
        if mode not in {"atomic", "outbox-first"}:
            return _remember(state, f"Unknown delivery mode: {mode}.")
        return _remember(
            replace(state, config=replace(state.config, delivery_mode=mode)),
            f"Delivery mode is now {mode}; reset before comparing scenarios.",
        )
    if kind == "set-recovery-policy":
        policy = str(action.value)
        if policy not in {"commander-visible", "automatic-after-proof"}:
            return _remember(state, f"Unknown recovery policy: {policy}.")
        return _remember(
            replace(state, config=replace(state.config, recovery_policy=policy)),
            f"Same-session recovery policy is now {policy}.",
        )
    if kind == "set-cadence":
        value = max(1, int(action.value or 1))
        return _remember(
            replace(state, config=replace(state.config, tick_seconds=value)),
            f"Prototype tick/sweep cadence set to {value} seconds.",
        )
    if kind == "set-stale-threshold":
        value = max(1, int(action.value or 1))
        return _remember(
            replace(state, config=replace(state.config, stale_after_ticks=value)),
            f"Stale-activity threshold set to {value} ticks.",
        )
    if kind == "set-unavailable-threshold":
        value = max(1, int(action.value or 1))
        return _remember(
            replace(
                state,
                config=replace(state.config, unavailable_after_sweeps=value),
            ),
            f"Unavailable-observation threshold set to {value} sweeps.",
        )
    if action.value:
        return _remember(state, str(action.value))
    return _remember(state, f"Unknown action: {kind}.")


def projection(state: PrototypeState) -> Projection:
    """Derive one Mission Work line without changing either source of truth."""

    effect_priority = {
        "reconcile-result": 0,
        "recover-same-session": 1,
        "wait-for-quiescence": 2,
        "inspect-only": 3,
    }
    severity_priority = {"high": 0, "medium": 1, "diagnostic": 2}
    open_attention = sorted(
        (
            attention
            for attention in state.attentions
            if attention.disposition == "open"
        ),
        key=lambda attention: (
            effect_priority.get(attention.next_effect, 4),
            severity_priority.get(attention.severity, 3),
            -attention.first_seen_tick,
        ),
    )
    status = state.canonical.status.replace("-", " ").title()
    backlog = len(state.world.events) - state.observer.event_cursor
    supervision_degraded = (
        state.observer.watcher_status != "active" or backlog > 0
    )
    if open_attention:
        first = open_attention[0]
        supervision_suffix = (
            f" · supervision backlog {backlog}"
            if supervision_degraded
            else ""
        )
        return Projection(
            headline=(
                f"{status} · {len(open_attention)} attention · "
                f"{first.kind.replace('-', ' ')}{supervision_suffix}"
            ),
            detail=first.detail,
            next_action=first.next_effect.replace("-", " "),
        )
    if supervision_degraded:
        return Projection(
            headline=(
                f"Supervision degraded · {backlog} observation"
                f"{'' if backlog == 1 else 's'} pending"
            ),
            detail=(
                f"Watcher is {state.observer.watcher_status}; cursor "
                f"{state.observer.event_cursor}/{len(state.world.events)} has "
                "not yet delivered an actionable or benign receipt."
            ),
            next_action=(
                "restart watcher"
                if state.observer.watcher_status == "crashed"
                else "retry observer"
            ),
        )
    if state.world.operation_phase == "waiting-local-inference-lease":
        return Projection(
            headline=f"{status} · waiting for Local Inference Lease",
            detail="Resource scheduling is visible but is not classified as dead work.",
            next_action="wait",
        )
    if state.canonical.status == "queued" and state.receipts:
        return Projection(
            headline="Queued · infrastructure recovery acknowledged",
            detail=state.receipts[-1].outcome,
            next_action="dispatch through the Orchestrator",
        )
    return Projection(
        headline=f"{status} · no operational attention",
        detail="Canonical and independent observations do not require intervention.",
        next_action="none",
    )


def invariant_results(state: PrototypeState) -> tuple[tuple[str, bool], ...]:
    """Return the small set of invariants the prototype is meant to expose."""

    attention_ids = [item.attention_id for item in state.attentions]
    incident_keys = [item.incident_key for item in state.attentions]
    correlation_ids = [item.correlation_id for item in state.intents]
    receipt_correlations = [item.correlation_id for item in state.receipts]
    cursor_prefix = [item.seq for item in state.observer.cursor_receipts]
    actionable_receipts_valid = all(
        receipt.disposition != "actionable"
        or (
            receipt.attention_id in attention_ids
            and any(
                intent.attention_id == receipt.attention_id
                for intent in state.intents
            )
            or any(
                attention.attention_id == receipt.attention_id
                and attention.next_effect not in {
                    "reconcile-result",
                    "recover-same-session",
                }
                for attention in state.attentions
            )
        )
        for receipt in state.observer.cursor_receipts
    )
    open_effect_attentions_have_intents = all(
        sum(
            intent.attention_id == attention.attention_id
            and intent.status in {"pending", "in-flight"}
            for intent in state.intents
        )
        == 1
        for attention in state.attentions
        if attention.disposition == "open"
        and attention.next_effect in {
            "reconcile-result",
            "recover-same-session",
        }
    )
    closed_attentions_have_no_live_intent = all(
        not any(
            intent.attention_id == attention.attention_id
            and intent.status in {"pending", "in-flight"}
            for intent in state.intents
        )
        for attention in state.attentions
        if attention.disposition != "open"
    )
    return (
        (
            "cursor covers one contiguous delivered prefix",
            cursor_prefix == list(range(1, state.observer.event_cursor + 1)),
        ),
        (
            "every advanced actionable event has durable attention/intent",
            actionable_receipts_valid,
        ),
        (
            "semantic incidents and effect correlations are unique",
            len(incident_keys) == len(set(incident_keys))
            and len(correlation_ids) == len(set(correlation_ids)),
        ),
        (
            "effect receipts are correlation-idempotent",
            len(receipt_correlations) == len(set(receipt_correlations)),
        ),
        (
            "every open automatic-effect attention has one usable intent",
            open_effect_attentions_have_intents,
        ),
        (
            "resolved or superseded attention has no live intent",
            closed_attentions_have_no_live_intent,
        ),
        (
            "observations spent no tokens and mutated no Mission state",
            state.metrics.model_turns == 0
            and state.metrics.observation_mission_mutations == 0,
        ),
        (
            "cursor never exceeds available source events",
            state.observer.event_cursor <= len(state.world.events),
        ),
    )


DEMO_SCRIPTS: dict[str, tuple[Action, ...]] = {
    "healthy": tuple(Action("healthy") for _ in range(5)),
    "normal": (
        Action("normal-completion"),
        Action("watch"),
    ),
    "missed": (
        Action("missed-completion"),
        Action("restart"),
        Action("apply"),
        Action("restart"),
        Action("apply"),
    ),
    "duplicate-observation": (
        Action("missed-completion"),
        Action("missed-completion"),
        Action("restart"),
        Action("watch"),
        Action("apply"),
    ),
    "invalid-result": (
        Action("invalid-result"),
        Action("watch"),
        Action("restart"),
        Action("apply"),
    ),
    "crash-atomic": (
        Action("set-delivery-mode", "atomic"),
        Action("arm-crash"),
        Action("dead-owner"),
        Action("watch"),
        Action("restart"),
        Action("apply"),
        Action("restart"),
        Action("apply"),
    ),
    "crash-outbox": (
        Action("set-delivery-mode", "outbox-first"),
        Action("arm-crash"),
        Action("dead-owner"),
        Action("watch"),
        Action("restart"),
        Action("apply"),
        Action("restart"),
        Action("apply"),
    ),
    "cursor-failure": (
        Action("set-delivery-mode", "outbox-first"),
        Action("arm-cursor-failure"),
        Action("dead-owner"),
        Action("watch"),
        Action("watch"),
        Action("apply"),
        Action("apply"),
    ),
    "dead-owner": (
        Action("dead-owner"),
        Action("watch"),
        Action("apply"),
    ),
    "restart-replay": (
        Action("dead-owner"),
        Action("watch"),
        Action("apply"),
        Action("restart"),
        Action("apply"),
    ),
    "rejected-proof-rearms": (
        Action("dead-owner"),
        Action("watch"),
        Action("pid-reuse"),
        Action("apply"),
        Action("dead-owner"),
        Action("watch"),
        Action("watch"),
        Action("apply"),
    ),
    "automatic-recovery": (
        Action("set-recovery-policy", "automatic-after-proof"),
        Action("dead-owner"),
        Action("watch"),
    ),
    "pid-reuse": (
        Action("pid-reuse"),
        Action("watch"),
        Action("sweep"),
        Action("apply"),
    ),
    "stale-output": (
        Action("stale-activity"),
        Action("sweep"),
        Action("contradictory-output"),
        Action("watch"),
    ),
    "stale-clears": (
        Action("stale-activity"),
        Action("sweep"),
        Action("healthy"),
    ),
    "attention-priority": (
        Action("stale-activity"),
        Action("sweep"),
        Action("dead-owner"),
        Action("watch"),
    ),
    "contradictory-output": (
        Action("contradictory-output"),
        Action("watch"),
    ),
    "unavailable": (
        Action("unavailable"),
        Action("sweep"),
        Action("sweep"),
        Action("restart"),
    ),
    "unavailable-restarts": (
        Action("unavailable"),
        Action("sweep"),
        Action("restart"),
    ),
    "unavailable-clears": (
        Action("unavailable"),
        Action("sweep"),
        Action("sweep"),
        Action("healthy"),
    ),
    "terminal-live": (
        Action("terminal-owner-live"),
        Action("watch"),
        Action("sweep"),
        Action("apply"),
    ),
    "canonical-supersedes": (
        Action("dead-owner"),
        Action("watch"),
        Action("normal-completion"),
        Action("watch"),
        Action("apply"),
    ),
    "inference-wait": (
        Action("inference-wait"),
        Action("sweep"),
        Action("sweep"),
    ),
}


def run_demo(name: str) -> PrototypeState:
    """Run one deterministic scenario from a clean state."""

    if name not in DEMO_SCRIPTS:
        raise KeyError(name)
    state = initial_state()
    for action in DEMO_SCRIPTS[name]:
        state = transition(state, action)
    return state
