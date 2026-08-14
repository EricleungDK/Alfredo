# Issue #71 — Versioned Host Execution Request/Receipt Boundary

## Outcome

Issue #71 adds one schema-versioned Python boundary for Local Agent and Shell host effects. Python still authorizes the exact effect, persists the caller's intent, invokes the provider, and reconciles the typed receipt into the existing canonical session or Shell Terminal projection.

## Contract

`albert_mvp.execution` defines:

- `ExecutionRequest`: exact argv, canonical working directory, effect-specific authority, sanitized environment, Bubblewrap filesystem boundary, bounded input, and timeout/output/resource limits. Shell parsing is rejected (`shell=False` is part of the contract).
- `ExecutionReceipt`: typed completed/failed/timed-out/output-limit/start-failed/cancelled/outcome-unknown result with process identity, error identity, output byte counts, and SHA-256 digests.
- `PythonExecutionProvider`: adapter to the existing bounded process runner, preserving Bubblewrap, environment sanitization, process supervision/cancellation, process-tree cleanup, resource limits, timeout, bounded output, and UTF-8 rejection. On POSIX it structurally validates the prepared Bubblewrap and `prlimit` argv, including namespace flags, private mounts, exact working-directory binding, declared read/write bindings, and resource-limit arguments, before the journal claim. Environment names and aggregate values are bounded by the provider contract.
- `ExecutionJournal` and `ExecutionCoordinator`: atomic, lock-serialized intent claim and receipt reconciliation. Exact terminal replay returns the stored receipt without provider invocation; changed request boundaries fail with `ExecutionReplayConflict`; dead owners or provider crash cuts become reconciliation-required `outcome-unknown`.

Raw stdout/stderr and model input are transient. The journal stores input/output digests and byte counts, not prompt or terminal bytes.

## Authority separation

Local Agent requests retain Mission/session revision, runner operation, Worktree Identity, and allowed-path authority. Shell requests retain Mission/command/correlation identity, command classification, requester, approval actor, working directory, requested paths, and access level. The shared provider does not authorize either path or mutate Mission state.

## Integration

- Local Agent command, model-inference, planned-command, and test-command paths use the provider after their existing worktree/sandbox/policy preparation.
- Shell Terminal uses the provider after existing command classification, approvals, Additional Path Grant checks, sandbox argv construction, and durable `executing` marker persistence. Canonical Shell metadata records request and receipt identities.
- Local Agent projects each terminal receipt into a bounded, redacted `LocalAgentSession.execution_receipts` history and replays journal receipts into that canonical session projection during startup. Cancellation refreshes the latest session revision before recording its receipt, so a concurrent projection cannot strand the durable cancellation. Shell inspection and mutation boundaries replay a durable receipt from the submitting Mission's ledger into the canonical terminal record if a crash occurred after provider completion but before terminal projection; terminal and chronology writes use one lock order.
- Mission startup and Shell inspection reconcile dead execution owners before projecting state. An uncertain effect remains `outcome-unknown`; retries never launch it automatically.
- Pre-effect provider failures are typed `start-failed` and never treated as uncertain effects. Journal writes fail closed on missing schema/receipt identity and fsync the containing directory after atomic replacement so the claim/receipt cut is durable across a crash.
- Existing one-process CLI and persistent transport argv envelopes remain unchanged. The Python backend remains the public authority consumed by the packaged desktop bridge.

## Verification

Focused contract/integration coverage: `python3 -m unittest tests.test_execution tests.test_execution_integration tests.test_runner_supervision` — 38 tests passed.

The existing broad Shell/Workspace suite was also exercised with `python3 -m unittest -q tests.test_workspace_snapshot`: 270 tests ran, with 20 failures, 60 errors, and 6 skips. The environment lacks trusted Bubblewrap, so real host-process tests fail closed; the remaining failures/errors include existing macOS `/private` path assumptions and unrelated dirty Issue #69/#70 work. The focused tests inject the bounded runner at the provider seam and verify successful receipts, exact replay, resource-limit propagation, authority separation, bounded environment validation, raw-output redaction, prepared-Bubblewrap validation before claim, canonical projection recovery, latest-revision cancellation, journal durability, and crash-cut no-replay behavior.
