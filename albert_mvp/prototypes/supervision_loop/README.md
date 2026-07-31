# PROTOTYPE — Attention-driven Local Agent supervision

This disposable logic prototype asks:

> Can an Alfredo-owned supervision loop combine canonical Local Agent state
> with independent runner observations, durably deliver actionable attention
> before advancing observer cursors, recover after watcher/restart faults, and
> remain silent and token-free for healthy no-change work without granting an
> observation Mission authority?

It is planning evidence for the Wayfinder ticket **Prototype Alfredo's
attention-driven Local Agent supervision loop**. It does not implement product
behavior, write runtime state, invoke a model, or change a real Mission.

## Mission Commander review

The human review surface is mounted inside the approved Variant C Focus desk.
From the repository root:

```bash
cd mission-control
npm run prototype:tree
```

Open:

```text
http://127.0.0.1:14874/?variant=C&review=supervision
```

`ISS-53` is selected by default. Choose a plain-language fault case, move
between its before/after/restart-or-receipt steps, and change the alert
persistence, recovery, and timing choices. The page prepares a review summary
without changing backend or Mission state.

## Engineering harness

From the repository root:

```bash
python3 -m albert_mvp.prototypes.supervision_loop
```

The terminal frame is reducer/fault evidence, not the Mission Commander review
surface. It always shows:

1. the authoritative canonical `SessionView`;
2. advisory runner/liveness facts;
3. the durable attention ledger and observer cursor;
4. typed Orchestrator intents and receipts;
5. the derived Mission Work line;
6. an approximation of current Python's startup-only dead-owner recovery; and
7. detection, duplicate-prevention, no-change, and token-use measurements.

Run every scripted fault comparison without the interactive shell:

```bash
python3 -m albert_mvp.prototypes.supervision_loop --demo all
```

Trace every intermediate state for one comparison:

```bash
python3 -m albert_mvp.prototypes.supervision_loop \
  --demo crash-outbox --trace
```

Useful hand-driven sequences:

- Healthy silence: press `h` repeatedly.
- Missed completion with receipt replay: `m`, `r`, `a`, `r`, `a`.
- Invalid/corrupt result blocks both completion and rerun: `i`, `w`, `r`, `a`.
- Atomic crash cut: `c`, `d`, `w`, `r`, `a`, `r`, `a`.
- Durable outbox-before-cursor cut: `v`, `c`, `d`, `w`, `r`, `a`, `r`, `a`.
- Cursor failure: `v`, `f`, `d`, `w`, `w`, `a`, `a`.
- Recovery-policy comparison: reset, press `g`, then `d`, `w`; exact
  quiescence proof applies the typed intent automatically in that variant.
- PID reuse: `p`, `w`, `b`, `a`.
- Stale activity plus contradictory prose: `s`, `b`, `o`, `w`.
- Unavailable observations: `u`, `b`, `b`, `r`.
- Unavailable grace survives restart: `u`, `b`, `r`.
- Canonical terminal with a live owner: `t`, `w`, `b`, `a`.
- Resource wait is not liveness failure: `l`, `b`, `b`.

`v` toggles between an atomic supervision-ledger commit and the fallback
outbox-first ordering for stores that cannot atomically persist attention plus
cursor. `g` toggles between Mission Commander-visible application of the typed
same-session recovery intent and deterministic automatic application after the
full identity/quiescence/result-absence proof. `x` switches between a compact
one-screen frame and full ledger detail.

One prototype tick/sweep defaults to 15 seconds, so the default thresholds are
shown with wall-clock equivalents. Cadence and thresholds are intentionally
adjustable:

```text
cadence 10
threshold stale 5
threshold unavailable 3
```

## Mission Commander review

The prototype is ready to resolve only after answering these choices:

1. Atomic attention-plus-cursor commit only, or also support the visible
   outbox-first fallback?
2. Automatically apply a same-session recovery intent after exact proof, or
   leave that typed intent visible for a Mission Commander action?
3. What sweep cadence and stale/unavailable wall-clock thresholds feel right?
4. Should multiple sources merge into one semantic incident and retain
   resolved/superseded records with their typed condition/effect receipt?
5. Does the ranked single Mission Work line make the most consequential open
   condition and pending action clear inside the approved Focus desk?
6. Does the persisted-result path correctly block rerun until typed result
   validation of its session, operation, worktree, and content succeeds?

## Current-Python comparison boundary

The comparison panel models the behavior visible in the current local code:
Mission load checks a `running` session's Python PID/start identity and directly
requeues the same session/worktree up to three times, then fails it. It has no
typed operational-attention record, observer cursor, or immutable runner
operation identity, and it does not reconcile an exact persisted result before
requeueing. The panel is a deliberately narrow contrast, not a complete
reimplementation of the current backend.

## Provisional labels

`Runner Observation`, `Observer Cursor`, `Reconciliation Sweep`, and
`Local Agent Attention Record` are prototype labels. They are not canonical
domain terms until the Mission Commander resolves the prototype.
