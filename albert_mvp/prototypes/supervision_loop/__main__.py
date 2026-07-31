"""Terminal driver for the disposable supervision-loop prototype."""

from __future__ import annotations

import argparse
import sys
import textwrap

from .model import (
    DEMO_SCRIPTS,
    Action,
    PrototypeState,
    initial_state,
    invariant_results,
    projection,
    run_demo,
    transition,
)


COLOR = sys.stdout.isatty()
BOLD = "\x1b[1m" if COLOR else ""
DIM = "\x1b[2m" if COLOR else ""
RESET = "\x1b[0m" if COLOR else ""


def _compact(value: str, width: int = 92) -> str:
    return textwrap.shorten(value, width=width, placeholder="…")


def _attention_lines(state: PrototypeState) -> list[str]:
    if not state.attentions:
        return ["  none"]
    lines: list[str] = []
    for item in state.attentions:
        receipt = (
            f" · resolution={item.resolution_receipt}"
            if item.resolution_receipt
            else ""
        )
        lines.append(
            "  "
            f"{item.attention_id} {item.disposition.upper()} · {item.kind} "
            f"· severity={item.severity} · latency={item.detection_latency_ticks} "
            f"tick(s){receipt}"
        )
        lines.append(
            "    "
            f"sources={','.join(item.sources)} · next={item.next_effect} "
            f"· seen@canonical-r{item.canonical_revision_seen}"
        )
    return lines


def _intent_lines(state: PrototypeState) -> list[str]:
    if not state.intents:
        return ["  none"]
    lines: list[str] = []
    for intent in state.intents:
        receipt = f" · receipt={intent.receipt_key}" if intent.receipt_key else ""
        lines.append(
            "  "
            f"{intent.intent_id} {intent.status.upper()} · {intent.kind} "
            f"· attempts={intent.attempt_count}{receipt}"
        )
        lines.append(f"    correlation={intent.correlation_id}")
        lines.append(
            "    "
            f"boundary={intent.mission_id_expected}/"
            f"{intent.session_id_expected} · "
            f"{intent.worktree_identity_expected} · "
            f"op={intent.operation_id_expected} · "
            f"pid={intent.runner_pid_expected}@"
            f"{intent.runner_start_identity_expected} · "
            f"group={intent.process_group_identity_expected}"
            + (
                f" · result={intent.result_digest_expected}"
                if intent.result_digest_expected
                else " · result=absent"
            )
        )
    return lines


def _render_compact(state: PrototypeState, *, clear: bool) -> str:
    prefix = "\033[2J\033[H" if clear else ""
    projected = projection(state)
    checks = invariant_results(state)
    passed = sum(1 for _, ok in checks if ok)
    runner = state.canonical.runner
    runner_label = (
        f"pid={runner.pid} · op={runner.operation_id}"
        if runner is not None
        else "none"
    )
    recovery_label = (
        "automatic"
        if state.config.recovery_policy == "automatic-after-proof"
        else "commander"
    )
    cursor_delivery = (
        (
            f"event {state.observer.cursor_receipts[-1].seq} → "
            f"{state.observer.cursor_receipts[-1].attention_id or 'benign'}"
        )
        if state.observer.cursor_receipts
        else "none"
    )
    attention_summary = "; ".join(
        (
            f"{item.attention_id}:{item.disposition}:{item.kind}"
            f"{'/' + item.resolution_receipt if item.resolution_receipt else ''}"
        )
        for item in state.attentions[-3:]
    ) or "none"
    intent_summary = "; ".join(
        (
            f"{item.intent_id}:{item.status}:{item.kind}"
            f"{'/' + item.receipt_key if item.receipt_key else ''}"
        )
        for item in state.intents[-2:]
    ) or "none"
    stale_seconds = (
        state.config.stale_after_ticks * state.config.tick_seconds
    )
    unavailable_seconds = (
        state.config.unavailable_after_sweeps * state.config.tick_seconds
    )
    lines = [
        f"{BOLD}PROTOTYPE — Alfredo Local Agent supervision{RESET}",
        (
            f"{DIM}Observations raise attention; only Orchestrator receipts "
            f"change Mission state.{RESET}"
        ),
        (
            f"{BOLD}Choose{RESET} mode={state.config.delivery_mode} · "
            f"recovery={recovery_label} · tick={state.config.tick_seconds}s · "
            f"stale≈{stale_seconds}s · unavailable≈{unavailable_seconds}s"
        ),
        "",
        f"{BOLD}Mission Work{RESET}  {projected.headline}",
        f"  {_compact(projected.detail, 105)}",
        f"  next={projected.next_action}",
        (
            f"{BOLD}Canonical{RESET} r{state.canonical.revision} "
            f"{state.canonical.status} · runner={runner_label} · "
            f"result={state.canonical.result_receipt or 'none'}"
        ),
        (
            f"{BOLD}Advisory{RESET} t{state.world.tick} owner={state.world.owner_signal} "
            f"group={state.world.process_group_signal} · {state.world.operation_phase} · "
            f"result={state.world.persisted_result or 'none'}/"
            f"{state.world.result_validation}"
        ),
        (
            f"{BOLD}Ledger{RESET} watcher={state.observer.watcher_status} · "
            f"cursor={state.observer.event_cursor}/{len(state.world.events)} · "
            f"delivery={cursor_delivery} · armed={state.observer.next_fault}"
        ),
        f"{BOLD}Attention{RESET} {attention_summary}",
        f"{BOLD}Intents{RESET}   {intent_summary}",
        (
            f"{BOLD}Current Python{RESET} {state.baseline.status} · startup effects="
            f"{state.baseline.startup_recovery_effects} · no typed attention"
        ),
        (
            f"{BOLD}Measure{RESET} sweep/probe={state.metrics.reconciliation_sweeps}/"
            f"{state.metrics.independent_probes} · attn="
            f"{state.metrics.attention_writes}/{state.metrics.attention_resolutions} "
            f"closed · effects={state.metrics.domain_effects} · dedupe="
            f"{state.metrics.duplicate_effects_prevented} · tokens="
            f"{state.metrics.model_turns}"
        ),
        f"{BOLD}Invariants{RESET} {passed}/{len(checks)} PASS · {_compact(state.last_message, 52)}",
        "",
        (
            f"{BOLD}Inject{RESET} h healthy · n normal · m missed · i invalid · "
            "d dead · p PID"
        ),
        (
            "       s stale · o prose · u unavailable · t terminal/live · l lease"
        ),
        (
            f"{BOLD}Drive{RESET}  w event · b sweep · c crash · f cursor · "
            "r restart · a apply"
        ),
        "       0 reset · q quit · x full state",
        (
            f"{BOLD}Choose{RESET} v delivery · g recovery · threshold/cadence commands · "
            "demo <name>"
        ),
    ]
    return prefix + "\n".join(lines)


def render(
    state: PrototypeState,
    *,
    clear: bool,
    verbose: bool = False,
) -> str:
    if not verbose:
        return _render_compact(state, clear=clear)
    if clear:
        prefix = "\033[2J\033[H"
    else:
        prefix = ""
    projected = projection(state)
    checks = invariant_results(state)
    passed = sum(1 for _, ok in checks if ok)
    runner = state.canonical.runner
    runner_label = (
        (
            f"pid={runner.pid} start={runner.start_identity} "
            f"group={runner.process_group_identity} op={runner.operation_id}"
        )
        if runner is not None
        else "none"
    )
    receipt = state.receipts[-1].receipt_key if state.receipts else "none"
    lines = [
        f"{BOLD}PROTOTYPE — Alfredo attention-driven Local Agent supervision{RESET}",
        (
            f"{DIM}Advisory observations may raise attention; only a typed "
            f"Orchestrator receipt may mutate canonical state.{RESET}"
        ),
        "",
        (
            f"{BOLD}Configuration{RESET}  delivery={state.config.delivery_mode} · "
            f"recovery={state.config.recovery_policy} · cadence="
            f"{state.config.tick_seconds}s · stale={state.config.stale_after_ticks} "
            f"ticks · unavailable={state.config.unavailable_after_sweeps} sweeps"
        ),
        "",
        f"{BOLD}Canonical SessionView (authoritative, read-only to observation){RESET}",
        (
            f"  {state.canonical.mission_id} / {state.canonical.work_id} / "
            f"{state.canonical.session_id}"
        ),
        (
            f"  revision={state.canonical.revision} · status={state.canonical.status} "
            f"· recovery_count={state.canonical.recovery_count} "
            f"· result_receipt={state.canonical.result_receipt or 'none'}"
        ),
        f"  runner={runner_label}",
        f"  worktree={state.canonical.worktree_identity}",
        "",
        f"{BOLD}Independent advisory world{RESET}",
        (
            f"  tick={state.world.tick} · owner={state.world.owner_signal} · "
            f"group={state.world.process_group_signal} · "
            f"worktree={state.world.worktree_signal}"
        ),
        (
            f"  phase={state.world.operation_phase} · "
            f"activity_age={state.world.tick - state.world.last_activity_tick} · "
            f"result={state.world.persisted_result or 'none'} · "
            f"validation={state.world.result_validation}"
        ),
        f"  diagnostic={state.world.diagnostic_output or 'none'}",
        "",
        f"{BOLD}Durable supervision ledger{RESET}",
        (
            f"  watcher={state.observer.watcher_status} · "
            f"cursor={state.observer.event_cursor}/{len(state.world.events)} "
            f"({state.observer.source_incarnation}) · "
            f"armed_fault={state.observer.next_fault}"
        ),
        f"  unavailable_streak={state.observer.unavailable_streak}",
        (
            "  latest cursor-delivery receipt="
            + (
                f"event {state.observer.cursor_receipts[-1].seq} → "
                f"{state.observer.cursor_receipts[-1].attention_id or 'benign'}"
                if state.observer.cursor_receipts
                else "none"
            )
        ),
        f"  {BOLD}Attention records{RESET}",
        *_attention_lines(state),
        f"  {BOLD}Typed intents / receipts{RESET}",
        *_intent_lines(state),
        f"  latest receipt={receipt}",
        (
            "  latest effect boundary="
            + (
                state.receipts[-1].boundary or "supervision-ledger condition receipt"
                if state.receipts
                else "none"
            )
        ),
        "",
        f"{BOLD}Mission Work projection (derived){RESET}",
        f"  {projected.headline}",
        f"  {_compact(projected.detail)}",
        f"  next={projected.next_action}",
        "",
        f"{BOLD}Current Python contrast (approximation of local code){RESET}",
        (
            f"  status={state.baseline.status} · restart recoveries="
            f"{state.baseline.recovery_count} · canonical effects="
            f"{state.baseline.startup_recovery_effects} · typed attention=none"
        ),
        f"  {_compact(state.baseline.last_message)}",
        "",
        f"{BOLD}Measurements{RESET}",
        (
            f"  sweeps={state.metrics.reconciliation_sweeps} · probes="
            f"{state.metrics.independent_probes} · event attempts="
            f"{state.metrics.event_attempts} · attention writes="
            f"{state.metrics.attention_writes} · dedupes="
            f"{state.metrics.attention_dedupes} · resolved="
            f"{state.metrics.attention_resolutions}"
        ),
        (
            f"  cursor advances={state.metrics.cursor_advances} · domain effects="
            f"{state.metrics.domain_effects} · duplicate effects prevented="
            f"{state.metrics.duplicate_effects_prevented} · restarts="
            f"{state.metrics.restarts}"
        ),
        (
            f"  model turns={state.metrics.model_turns} · Activity Journal noise="
            f"{state.metrics.activity_journal_entries} · healthy UI emissions="
            f"{state.metrics.healthy_user_emissions}"
        ),
        f"  invariants={passed}/{len(checks)} PASS",
        "",
        f"{BOLD}Last transition{RESET}  {_compact(state.last_message)}",
        "",
        (
            f"{BOLD}Inject{RESET} [h] healthy  [n] normal completion  [m] missed "
            "[i] invalid result  [d] dead owner  [p] PID reuse"
        ),
        (
            "       [s] stale  [o] output says DONE  [u] probe unavailable  "
            "[t] terminal+live  [l] inference wait"
        ),
        (
            f"{BOLD}Drive{RESET}  [w] watch event  [b] backstop sweep  [c] arm crash  "
            "[f] arm cursor failure"
        ),
        (
            "       [r] restart  [a] apply/replay typed intent  [v] toggle delivery "
            "[g] toggle recovery  [x] compact/full  [0] reset  [q] quit"
        ),
        (
            f"{DIM}Also: demo <name>; threshold stale <n>; "
            f"threshold unavailable <n>; cadence <seconds>; help{RESET}"
        ),
    ]
    return prefix + "\n".join(lines)


SHORT_ACTIONS = {
    "h": Action("healthy"),
    "n": Action("normal-completion"),
    "m": Action("missed-completion"),
    "i": Action("invalid-result"),
    "d": Action("dead-owner"),
    "p": Action("pid-reuse"),
    "s": Action("stale-activity"),
    "o": Action("contradictory-output"),
    "u": Action("unavailable"),
    "t": Action("terminal-owner-live"),
    "l": Action("inference-wait"),
    "w": Action("watch"),
    "b": Action("sweep"),
    "c": Action("arm-crash"),
    "f": Action("arm-cursor-failure"),
    "r": Action("restart"),
    "a": Action("apply"),
    "0": Action("reset"),
}


def _handle_line(state: PrototypeState, raw: str) -> tuple[PrototypeState, bool]:
    command = raw.strip()
    if command in {"q", "quit", "exit"}:
        return state, True
    if command in {"help", "?"}:
        return transition(
            state,
            Action(
                "unknown",
                "Use one-letter controls, demo <name>, or threshold <kind> <n>.",
            ),
        ), False
    if command in SHORT_ACTIONS:
        return transition(state, SHORT_ACTIONS[command]), False
    if command == "v" or command == "mode":
        mode = (
            "outbox-first"
            if state.config.delivery_mode == "atomic"
            else "atomic"
        )
        return transition(state, Action("set-delivery-mode", mode)), False
    if command == "g" or command == "recovery":
        policy = (
            "automatic-after-proof"
            if state.config.recovery_policy == "commander-visible"
            else "commander-visible"
        )
        return transition(state, Action("set-recovery-policy", policy)), False
    if command.startswith("demo "):
        name = command.removeprefix("demo ").strip()
        try:
            return run_demo(name), False
        except KeyError:
            choices = ", ".join(DEMO_SCRIPTS)
            return transition(
                state,
                Action("unknown", f"Unknown demo {name}; choose {choices}."),
            ), False
    if command.startswith("threshold "):
        parts = command.split()
        if len(parts) == 3 and parts[2].isdigit():
            if parts[1] == "stale":
                return transition(
                    state,
                    Action("set-stale-threshold", int(parts[2])),
                ), False
            if parts[1] == "unavailable":
                return transition(
                    state,
                    Action("set-unavailable-threshold", int(parts[2])),
                ), False
    if command.startswith("cadence "):
        parts = command.split()
        if len(parts) == 2 and parts[1].isdigit():
            return transition(
                state,
                Action("set-cadence", int(parts[1])),
            ), False
    return transition(state, Action(command or "empty")), False


def _trace_row(step: int, label: str, state: PrototypeState) -> str:
    open_attention = [
        item.kind for item in state.attentions if item.disposition == "open"
    ]
    pending_intents = [
        item.kind for item in state.intents if item.status in {"pending", "in-flight"}
    ]
    return (
        f"  {step:02d} {label:<24} canonical={state.canonical.status:<14} "
        f"owner={state.world.owner_signal:<11} cursor="
        f"{state.observer.event_cursor}/{len(state.world.events)} "
        f"attention={','.join(open_attention) or '-'} "
        f"intent={','.join(pending_intents) or '-'} receipts={len(state.receipts)} "
        f"view={projection(state).headline}"
    )


def _print_demo(name: str, *, trace: bool = False) -> bool:
    if trace:
        state = initial_state()
        trace_lines = [_trace_row(0, "start", state)]
        for step, action in enumerate(DEMO_SCRIPTS[name], start=1):
            state = transition(state, action)
            trace_lines.append(_trace_row(step, action.kind, state))
    else:
        state = run_demo(name)
        trace_lines = []
    checks = invariant_results(state)
    open_attention = [
        item.kind for item in state.attentions if item.disposition == "open"
    ]
    latencies = [item.detection_latency_ticks for item in state.attentions]
    print(f"{BOLD}{name}{RESET}")
    print(f"  actions: {' → '.join(action.kind for action in DEMO_SCRIPTS[name])}")
    for line in trace_lines:
        print(line)
    print(
        "  current Python: "
        f"status={state.baseline.status}, effects="
        f"{state.baseline.startup_recovery_effects}"
    )
    print(
        "  proposed loop: "
        f"status={state.canonical.status}, cursor="
        f"{state.observer.event_cursor}/{len(state.world.events)}, "
        f"open_attention={open_attention or ['none']}, "
        f"receipts={len(state.receipts)}, tokens={state.metrics.model_turns}"
    )
    print(
        "  measurements: "
        f"sweeps={state.metrics.reconciliation_sweeps}, "
        f"probes={state.metrics.independent_probes}, "
        f"attention={state.metrics.attention_writes}, "
        f"dedupes={state.metrics.attention_dedupes}, "
        f"detection-latency={max(latencies) if latencies else 0} tick(s), "
        f"duplicate-effects-prevented="
        f"{state.metrics.duplicate_effects_prevented}"
    )
    failed = [label for label, ok in checks if not ok]
    print(f"  invariants: {'PASS' if not failed else 'FAIL — ' + ', '.join(failed)}")
    print(f"  projection: {projection(state).headline}")
    print()
    return not failed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run the throwaway Alfredo supervision-loop prototype.",
    )
    parser.add_argument(
        "--demo",
        choices=("all", *DEMO_SCRIPTS.keys()),
        help="Run deterministic fault scenarios without entering the TUI.",
    )
    parser.add_argument(
        "--mode",
        choices=("atomic", "outbox-first"),
        default="atomic",
        help="Initial attention/cursor persistence mode.",
    )
    parser.add_argument(
        "--no-clear",
        action="store_true",
        help="Do not clear the terminal between interactive frames.",
    )
    parser.add_argument(
        "--trace",
        action="store_true",
        help="Show every state transition in a scripted demo.",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Start the interactive TUI with the full state frame.",
    )
    args = parser.parse_args(argv)

    if args.demo:
        names = DEMO_SCRIPTS if args.demo == "all" else (args.demo,)
        return (
            0
            if all(_print_demo(name, trace=args.trace) for name in names)
            else 1
        )

    state = initial_state(delivery_mode=args.mode)
    clear = sys.stdout.isatty() and not args.no_clear
    verbose = args.verbose
    if not sys.stdin.isatty():
        print(render(state, clear=False, verbose=verbose))
        print("\nInteractive input requires a terminal; use --demo all for a batch run.")
        return 0

    while True:
        print(render(state, clear=clear, verbose=verbose), end="\n\n")
        try:
            raw = input("> ")
        except (EOFError, KeyboardInterrupt):
            print()
            return 0
        if raw.strip() == "x":
            verbose = not verbose
            continue
        state, done = _handle_line(state, raw)
        if done:
            return 0


if __name__ == "__main__":
    raise SystemExit(main())
