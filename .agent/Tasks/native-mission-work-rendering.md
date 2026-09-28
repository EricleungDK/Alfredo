# Native Mission Work rendering cost

Date: 2026-09-27  
Status: active; validated baseline captured, lazy projection and exclusive panel rendering implemented, focused verification underway.

## Authority and current evidence

The current user requests a responsive Rust multi-agent terminal and continued
regression fixes, with parallel agents and usable previews. Fresh authenticated
September27 reads of [parent #56](https://github.com/EricleungDK/Alfredo/issues/56)
and [tree #63](https://github.com/EricleungDK/Alfredo/issues/63) require canonical
work inspection, selected-only detail and evidence-qualified performance claims.
The native rewrite is explicitly authorized by the current user; older desktop
shell restrictions do not negate that request.

Prior architecture and limitations are recorded in the
[native foundation](../Reports/2026-09-13-rust-terminal-foundation.md),
[Mission Work tree evidence](../Reports/2026-09-27-native-mission-work-tree.json)
and [recovery preview evidence](../Reports/2026-09-27-check-result-recovery.json).
The tree report identified eager activity formatting as an unmeasured concern.

Independent read-only review found two concrete paths: `ui::work_inspector`
collects all matching `activity::entries` before displaying three summaries;
`ui::draw_inner` renders that inspector before higher-precedence panels cover it.
The tree projection itself is already cached by task/scope revision and view.
A proposed valid maximum-history fixture is one task proposal followed by4095
policy changes with small argv, within the4096 receipt/4MiB journal. Validate this
fixture against the real store before relying on it. No new timing was measured.

## Bounded investigation and implementation

1. Record identical-fixture release-mode redraw cohorts for the actual Mission Work
   inspector, evidence and Activity at normal and32x10 sizes. Record source hashes,
   fixture validity/size, cold/warm samples and p50/p95 separately. No live model.
2. Make recent activity consume at most three matching formatted entries while
   preserving full Activity behavior and canonical matching rules. A lazy projection
   may reuse the current formatter; do not create a second history authority.
3. Construct/render only the visible panel, retaining current precedence: planner,
   evidence, Activity, scope, inspector. Keep exact selection, geometry, scroll bounds
   and composer behavior. No persistence change or tree cache rewrite is presumed.
4. Compare exact visible buffers and canonical unchanged state across normal and
   narrow modes, both page directions and overlay transitions. Preserve revision,
   correlation and task identity, Plan membership, compound review/repair events and
   repair-resolution ancestry even with unrelated receipts interleaved.
5. Re-run the same controlled cohorts and installed PTY suite. Report measured
   rendering changes separately from model/server latency or whole-product speed.
   If rendering cost is immaterial, retain only justified small simplifications.

No whole-product speed claim may follow from a formatter microbenchmark. Avoid
flaky duration thresholds in ordinary tests; behavior and bounded work should be
verified at the appropriate public rendering/projection seams. Full Activity
caching, chronology redesign, model defaults and schema changes require separate
evidence and are outside this slice.

## Delegation and handoff

Root owns the controlled baseline/final measurements, integration, Cargo, installed
archive and preview. After agreeing projection API and panel precedence, separate
agents can own `activity.rs` projection plus semantic tests, `ui.rs` panel selection
plus rendering regressions, and independent review/acceptance. Root retains sole
Cargo ownership. Freeze packaged sources before building an immutable candidate.
Keep `dist/preview-2026-09-27-recovery` usable until a successor is verified.

This slice does not complete Mission Draft/Issue Graph import, automatic runner
recovery, retirement, model qualification, cross-platform or production acceptance.
