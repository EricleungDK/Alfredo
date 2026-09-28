# Native recovery after a recorded check result

Date: 2026-09-27  
Status: verified bounded slice; local human preview shipped. Broader runner and production gaps remain open.

## Authority and user journey

The active user requests a reliable Rust multi-agent terminal, regression fixes
and usable previews. Fresh authenticated reads on September 27 of
[#65](https://github.com/EricleungDK/Alfredo/issues/65),
[#71](https://github.com/EricleungDK/Alfredo/issues/71) and
[parent #56](https://github.com/EricleungDK/Alfredo/issues/56) require reconciliation
of late results without repeating uncertain effects. Their desktop completion
does not establish native parity. Native child order and the #65 blocker identify
#63 as the preceding supervision slice.

A worker can finish its approved check and then lose the terminal process before
`evidence.json` is saved. The task remains Running. Before this slice, recovery accepted intact final evidence or a proven interruption
before check launch; an existing launch intent without final evidence remained unknown.
The returned execution receipt was held only in memory across worker finalization.

This slice records that terminal check result durably and lets explicit recovery
acknowledge **Failed: interrupted after check; candidate not finalized**. Even a
zero-exit check cannot establish completed worker finalization or acceptance.

## Existing boundaries and compatibility

The [native foundation report](../Reports/2026-09-13-rust-terminal-foundation.md)
records the original isolated worker/evidence and explicit recovery architecture.

Read `run_boundary.rs`, the execution closure in `worker.rs`, `TaskStore::recover`,
the shared `execution.rs` provider and the
[precheck recovery evidence](../Reports/2026-09-20-precheck-recovery.json).
Existing schema-1 start/check markers bind task, run and baseline and use exclusive
creation plus file/directory synchronization. Their absence/validity requirements
must remain conservative.

Preserve start markers and intact final evidence. A new, explicitly versioned check
intent must bind the full authorized execution request and its canonical digest
before invoking the provider. Old check intents cannot gain recovery eligibility
because they lack that binding. These are separately versioned run artifacts;
do not presume a task or conversation schema migration.

## Implementation contract

1. Validate and bound a request-bearing check intent against the canonical Mission,
   task/run/baseline, approved files/check argv, deterministic managed worktree and
   fixed sandbox/environment/resource policy. Version its builder/validation
   contract so later defaults or host mount changes cannot reinterpret history.
2. Immediately after provider return, inside the blocking execution closure and
   before async continuation, save an immutable terminal-result artifact bound to
   the exact intent bytes, request identity/digest and actual receipt. An artifact
   publication failure prevents subsequent success claims.
3. Validate receipt schema, effect/request/provider binding, terminal status
   combinations, output byte counts/hashes and bounds. Successful deserialization
   and the current `rust-shadow` label alone are insufficient. Refuse executing,
   unknown or reconciliation-required results. Keep any shared-provider change
   narrow and rerun its compatibility tests.
4. Reject symlinks/special files, bound serialization before exclusive creation,
   synchronize file and parent, and never overwrite existing intent/result/evidence.
   Partial artifacts remain uncertainty. Determine independent size ceilings from
   supported request/receipt maxima rather than assuming the old 4 KiB marker cap.
5. Under the stopped worker owner lock, prefer valid final evidence and preserve
   malformed existing evidence unchanged. With absent final evidence and a verified
   terminal checkpoint, publish Failed interruption evidence retaining the check
   receipt/output, with no reconstructed patch or candidate. Reuse deterministic
   `finish:<run>` reconciliation and exact command/run identity.
6. Expose a specific stopped-after-check recovery explanation in task inspection.
   Recovery does not run a model, check, Git command or old worker. A repair is new
   separately approved work; dependencies remain blocked on the failed original.

The private artifacts live outside the worker-mounted worktree. Digests detect
corruption and mismatched substitution; they do not authenticate against a same-user
actor who can coherently replace the entire private state. Owner release and a
terminal check receipt do not prove every later Git/model/helper process is gone.
No process signaling, automatic respawn, worktree reuse, retirement or complete
Runner Quiescence is authorized by this result.

## Verification and delegation

Root coordinates Cargo and installed acceptance. Separate owners can implement the
artifact/validation boundary, worker/recovery integration and focused crash tests
after agreeing APIs and file boundaries. Freeze ownership before the full pipeline.

- Use a test-only subprocess fixture invoking the same production execution and
  checkpoint helper with a side-effect-counted check. Signal a deterministic cut,
  kill the fixture and recover from a fresh store; no shipping fault switch or
  sleep-based timing assertion.
- Cover before launch, during execution, before checkpoint completion, after
  checkpoint and after final evidence/receipt. Prove one check invocation and one
  Finish across restart and concurrent recovery.
- Refuse wrong task/run/Mission/baseline/request/path, active ownership, corrupt or
  unsupported artifacts, oversized/truncated output, uncertain cleanup and damaged
  existing evidence, preserving bytes on refusal. Cover legacy markers and intact
  final evidence explicitly.
- Inject publication failure and prove no success claim. A normal full-worker HTTP
  fixture must bind the actual checkpoint to its final evidence.
- Installed PTY may construct a crash state from a genuine installed-worker
  checkpoint and captured Running snapshot, then recover and inspect its retained
  output at normal size and 32x10. Label this as constructed state; the subprocess
  fixture supplies actual process-death evidence.
- Run focused recovery/provider/worker tests, full native regressions, formatting,
  strict Clippy and all installed archive suites. No live model call is necessary.

The installed acceptance preparation also reproduced a keyboard paging regression
at32x10: fixed ten-row task paging skips inspector rows when only two are visible.
This run includes viewport-derived task/evidence paging, with a real PageDown RED
and regression that preserves exact task state and reaches model/output details.

The preceding [Mission Work tree slice](native-mission-work-tree.md) and its
preview were verified before this implementation began. Full automatic
runner recovery, Mission Draft/Issue Graph parity, retirement, model qualification
and production launch remain separate requirements in the regression inventory.


## Verified checkpoint

[Implementation evidence](../Reports/2026-09-27-check-result-recovery.json) records
396 full native passes/8 opt-in skips, followed by8 final artifact/crash checks
and strict Clippy/formatting. The final added process cuts are test-only; both real
provider-return/pre-create and actual empty-file/pre-write cuts preserve uncertainty.
All7 installed checks pass, including genuine installed checkpoint binding, constructed
missing-finalization recovery and real32x10 paging. Counts overlap. No live model ran.

The local preview is `alfredo-tui/dist/preview-2026-09-27-recovery`; its archive
and all52 source/6 payload hashes were verified. Run from the repository:

```bash
./alfredo-tui/dist/preview-2026-09-27-recovery/alfredo-tui --model qwen3:14b --state-dir "$HOME/.local/state/alfredo-preview"
```

F2 opens Mission Work, F3 opens selected evidence, PageUp/PageDown scrolls the
focused panel and Ctrl+Q quits. The prior preview remains intact.
