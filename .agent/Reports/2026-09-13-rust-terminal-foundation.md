# Rust terminal foundation — 2026-09-13

## Scope and cause

The current user goal requests a ralph-tui-like Rust multi-agent terminal, repairs
for disconnection/response/layout problems, complete regression coverage and
GitHub launch readiness. Inspection found a React/Python desktop with Rust bridge
and host effects, not an existing Rust terminal. This is a new product migration;
the prior desktop's green gates cannot establish its completion.

Plan: [Rust terminal migration](../Tasks/rust-terminal-migration.md).
Implementation: [`alfredo-tui/`](../../alfredo-tui/README.md).
Architecture: [project architecture](../System/project_architecture.md).

## Dependency notice packaging checkpoint

The Linux candidate now includes DEPENDENCIES.json and THIRD_PARTY_NOTICES.txt,
covering 183 target-filtered resolved third-party packages, including build/dev
dependencies. Original cached crate archives are checked against Cargo.lock;
nested license/notice files retain exact bytes and individual range/digest records.
Missing notices, unsupported sources, altered archives and unsafe paths refuse
packaging. The installed verifier requires all six members and their checksums,
checks locked package identities, and verifies individual notice digests.

Nine offline fixture tests pass, including inner notice corruption with recomputed
outer checksums. Fresh release packaging and installed PTY acceptance pass (two
tests, 7.011 seconds). Candidate:
`/tmp/alfredo-notices-YmAmch/candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`,
SHA-256 `eb643c45a5adbe0e45606558ca1d10674f961d3ccaf983129ea5b44066529009`.
Logs: `/tmp/alfredo-notices-{package,installed}.log`. CI now includes the fixture
suite, but remote execution remains unobserved. Rust application source did not
change in this checkpoint; the prior 157-test result remains its latest full run.

The inventory is deliberately conservative, not a linked-code SBOM, compatibility
judgment or publisher signature. Alfredo’s own license preference is pending; no
license has been selected on the owner’s behalf. Security/dependency review and
full product/launch acceptance remain open. No commit or publication occurred.

## Saved reading anchors and MIT selection checkpoint

Conversation v4 now persists bounded logical-line/wrapped-row anchors while leaving
pending navigation transient. A red regression lost HISTORY_032 after hidden output
and restart; the same test now retains it in a different window geometry. Existing
v1/v2/v3 snapshots remain readable with numeric-offset fallback, and upgrades retain
exact version-named backups. Tests cover v3 backup conflicts, invalid anchors and
old schemas carrying new anchors. Interrupted requests still never replay. Width
reflow clamps the wrapped row; exact character-level anchoring remains open.

The owner explicitly chose MIT. Root LICENSE and Rust crate metadata now record MIT;
the seven-member candidate includes and fingerprints LICENSE alongside the existing
third-party notices. The standard text was checked against the
[OSI MIT license](https://opensource.org/license/mit). Copyright uses the repository's
configured author name, EricleungDK, and Alfredo contributors. Dependency terms remain
separate; this does not complete dependency compatibility/security qualification.

Full native regression: 159 passed, three optional ignored; strict Clippy and release
pass. Nine archive-integrity fixtures pass after correcting a test-verifier tuple-key
edit introduced when adding LICENSE. Installed PTY acceptance passes (two tests,
6.226 s), including scrolling, quitting and restoring the same older passage in a
new process. Candidate:
`/tmp/alfredo-anchor-final-TezzRe/candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`,
SHA-256 `16e0cfe5d824265733cd351aa20ca0f494075088cb1ae365b71b3a88a0b36145`.
Logs `/tmp/alfredo-anchor-{red,focused,full,clippy,package-final,installed}.log`.
Earlier candidate was superseded after an installation-document correction. No
browser code changed in this checkpoint; prior browser evidence remains scoped to
those legacy paths. Complete graph/formation/action chronology, qualification,
retirement and launch acceptance remain open. No commit, push or publication.

## Implemented behavior

- Native Rust terminal with a session list, transcript, bounded Unicode/paste
  composer, keyboard session switching, scrolling, narrow-layout fallback, and
  event-driven redraw. No model request blocks terminal input.
- Direct asynchronous Ollama chat with loading/idle/connect/total deadlines,
  bounded NDJSON frames and output, explicit stream errors and completion markers.
- Eight independent conversation slots and a bounded event channel; request
  attempts bind updates so late cancellation/retry events cannot cross sessions.
- Partial output survives failures/cancellation. Only an explicit retry replaces
  the incomplete turn. No automatic inference or effect replay.
- RAII terminal restoration on ordinary exit/error and Ratatui panic restoration;
  quit aborts all client requests. Closing a request does not prove server unload.
- Locked Rust crate, documented local commands, and Ubuntu GitHub regression CI.
- Durable Rust task queue with workspace/mission namespaces, sequential task ids,
  acyclic earlier-task dependencies, explicit approval/cancellation, bounded
  receipt replay validation, OS locking and synced atomic JSON publication.
- Async terminal task commands: `/task`, `/after`, `/approve`, `/cancel-task`,
  `/tasks`, `/chat`, `/refresh`, `/retry-task`, plus F2 projection switching.
  Task state changes only after its storage receipt. Approval is scheduling intent
  and cannot authorize future command/path effects without a new explicit policy.

## Verification evidence

- Rust deterministic suite: 26 passed (session behavior, provider HTTP fixtures,
  layout buffers, CLI preflight and ten durable task tests). Optional live test is
  excluded unless explicitly requested. Final formatting and strict Clippy pass.
- Final formatting, all deterministic Rust tests, strict
  `cargo clippy --all-targets -- -D warnings`, and PTY acceptance pass after the
  event-driven redraw optimization.
- Linux PTY: passed actual binary startup, stalled first request, responsive new
  session with independent completion, cancellation, quit and termios restoration.
  Both debug and optimized release binaries pass. The locked release build and
  release CLI help/version smoke also pass; generated binary is approximately 4 MiB.
  The final journey additionally creates and approves a task, closes the process,
  starts a new process in the same namespace, verifies restored approval, and
  proves reads do not advance revision. Both current debug and release binaries pass.
  The harness now reconstructs visible cells from the CSI controls emitted by the
  fixture instead of searching raw diff output for whole words. The restart test
  also caught and drove a real quit fix: read-only refresh does not block exit;
  only a pending write prompts for a second quit with outcome-unknown wording.
- Real local Ollama `qwen2.5-coder:14b`: replied `READY`; first content 16.9539 s,
  terminal marker 16.9953 s. This is one live observation, not a benchmark or speed
  improvement claim. Initial sandbox curl failed; permitted probe found the server.
- Existing Python suite: 797 tests ran, 3 optional skips, no failures (221.151 s).
- Existing Rust no-desktop bridge suite: 65 passed, one ignored subprocess fixture.
- Legacy frontend: sandbox run failed with loopback `EPERM` and subprocess-related
  fixture failures; stopped that run and launched the permitted verification.
  Final permitted result: 324 passed / one failed in 567.03 s. The release journey
  cannot inspect `/proc/372/cwd` (systemd, EACCES) before retirement. Tool session
  85942 completed with exit 1; `/tmp/alfredo-rust-migration-frontend-permitted.log`
  retains the full result. No retirement guard or assertion was weakened.
  TypeScript typecheck passes. Brief overlap occurred while stopping the sandbox
  run; no release packaging success may be inferred from either run.
- Documentation standards validator: A (97.8%); historical orphan/staleness
  findings remain. Current terminal plan/report/architecture/readme are linked.

New HTTP regressions initially failed at socket creation under the sandbox, before
exercising the provider. They passed with loopback permission. Dependency compile
also identified Ratatui's opt-in wrapped-line measurement feature; it is explicitly
enabled and used by the Unicode/reflow regression.

## Coding worker continuation

Rust now executes actual coding work from explicit file/check policies and fresh
approval. It claims each run durably before effects, creates a detached Git worktree
from committed HEAD, asks Ollama for a bounded structured file plan, validates every
path before writing, and runs the approved argv through the existing Rust execution
provider. Bubblewrap isolates host files and networking, mounts system tools
read-only, protects `.git`, and limits child resources/output/time. Evidence binds
run identity, baseline, diff and check receipt; completion and review verify its
digest. Acceptance records a decision without merging or committing the patch.

`/permit`, `/run`, `/evidence`, `/accept` and `/reject` expose this workflow. Up to
four workers per terminal run outside input/rendering. Cancellation waits for a
terminal receipt, late snapshots cannot roll back current revision, and replayed
claims never rerun effects. State schema v2 retains the namespace, reads v1 and
backs up its exact original bytes before mutation; old approvals gain no policy.

The live model initially returned fenced JSON despite a JSON-only prompt. Retained
raw response identified that cause; enabling Ollama's structured-output schema
and including the approved check fixed the request without weakening validation.
The real qwen2.5-coder:14b then changed two files and passed its actual sandbox check
in 2.1668 seconds. The original workspace was unchanged. This warm sample establishes
connectivity and a coding path, not comparative speed or sustained reliability.

Final verification for this continuation:

- Terminal Rust: 47 passed, two optional live tests ignored in the default suite.
  The live coding test was separately run and passed.
- Formatting, strict all-target Clippy and locked release build pass.
- Debug and release PTY journeys pass conversation concurrency/cancel, restart,
  explicit policy/approval, coding/check, evidence display and acceptance while
  preserving the original workspace and restoring terminal settings. Final release
  journey: 0.822 seconds against the deterministic HTTP fixture.
- Migration test proves exact v1 backup and no inferred execution permission.
  Worker tests cover denied model paths, bare approval, cancellation, evidence
  tampering, claim replay and real host-file/network/metadata isolation.
- Shared execution provider compatibility: 65 legacy Rust tests passed, one ignored;
  Python execution discovery ran 83 tests with one optional skip and no failures.
  Logs: `/tmp/alfredo-worker-rust-tests.log`, `/tmp/alfredo-worker-legacy-rust.log`,
  `/tmp/alfredo-worker-python-execution.log` (local ephemeral evidence).
- Ubuntu CI now installs Bubblewrap and triggers for the shared provider source;
  remote CI has not run. The binary includes that source at build time without
  requiring the desktop/Python backend at runtime.

## Live progress continuation

Active tasks now show the worker's current preparation/model/write/check/evidence
stage, elapsed time in that stage, total elapsed time, received model bytes and
first-content latency measured from request dispatch. A bounded Tokio watch value
per worker coalesces updates without blocking execution on rendering. Timers redraw
once per second during silence. Observations are process-local and never establish
success or modify durable task receipts; a restarted terminal relies on the saved
claim. Cancellation remains visible until the worker's terminal result is received,
then its progress observer is removed.

The deterministic delayed-model regression renders wide and narrow terminals,
asserts visible waiting without success, requests cancellation and waits for the
actual durable Cancelled result. Existing real-worker tests also verify zero bytes
before model content and observed byte/latency data after successful execution.
Current Rust suite: 48 passed, two optional live tests ignored; formatting and strict
Clippy pass. Debug PTY passed (1.289 s). Final release verification is recorded in
the active orchestration context. Test log: `/tmp/alfredo-progress-tests.log`.

## Retained-result recovery continuation

New workers hold a per-task OS lock from before the durable claim through result
publication. Refresh can distinguish a live owner, a stopped owner with valid
retained evidence, and an uncertain/unmarked legacy claim. `/recover ID` acquires
the released lock, validates retained evidence and records the missing Finish
receipt. It never calls Git, the model or the check. A repeated recovery validates
the already acknowledged evidence and returns without another mutation. Workers
also recognize their own already-published Finish receipt after a lost save response.

Process-death regression starts a real child owning the lock, writes a saved failed
result, proves recovery refuses while the owner lives, kills the child and recovers
exactly once. Separate cases preserve missing/truncated evidence, reject unsupported
success claims, and prevent a new launch from creating ownership for an old running
claim. A reopened-terminal test renders the recovery action and acknowledges its
saved result through the command adapter.

Parallel verification exposed immediate store-lock rejection during brief
contention; serial recovery passed. A focused test holding the lock for 40 ms
reproduced the rejection. Inspection also identified close-only lock release across
concurrent fork/exec: an inherited open-file description can retain the lock. A
transaction guard now explicitly unlocks before closing. Store transactions also
wait at most 250 ms, polling every
2 ms, while worker-owner checks stay nonblocking. This wait runs outside terminal
input/rendering. Final verification: 53 Rust tests passed, three ignored live/subprocess fixtures;
formatting, strict Clippy, locked release build and release PTY passed (0.821 s).
The subprocess fixture was exercised by its parent process-death test. Documentation
validation: A (98.2%). Log: `/tmp/alfredo-recovery-tests.log`.

This recovers saved terminal evidence only. Missing evidence does not establish
child quiescence or safe retry. Original claims/artifacts remain for inspection;
no uncertain effects are rerun and legacy ownership is never inferred.

## Keyboard task navigation continuation

The task view now replaces the conversation list with a compact task list and
renders one selected task's details/progress. Up/Down wraps through task identities;
background snapshots preserve that identity. F3 opens verified evidence. Task
actions can omit their ID to target the current selection, resolved once at command
submission through the existing revision/receipt path. Navigation does not perform
an action and is held while a storage request is pending. Selecting another task
clears prior evidence and resets scrolling. Task/evidence PageUp/PageDown direction
was corrected to match their top-origin scroll offsets.

The new regression uses 31 tasks to verify selected-target approval, stable
selection after an external proposal, wide/narrow rendering and evidence reset.
Full suite: 54 passed, three ignored live/subprocess fixtures. The release PTY
journey opens evidence with actual F3 input and accepts the selected task using
`/accept`; it passed in 0.826 seconds. Formatting, strict Clippy and locked release
build passed. Final heading/help focused tests, strict Clippy and release rebuild also pass;
final release PTY passed in 0.827 seconds. Documentation validation: A (98.2%).

## Prompt editing continuation

Prompt drafts now support Left/Right, Home/End, Backspace/Delete, Ctrl+W word
removal, Ctrl+U clear and Shift+Enter newline when the terminal distinguishes it.
Insertion/paste occurs at the cursor; sessions retain independent draft positions.
Movement/deletion operates on grapheme clusters, including combining accents and
joined emoji. Paste strips control characters and never truncates an accepted
cluster at the 16 KiB bound. The single-line viewport measures terminal-cell widths,
keeps a visible cursor in narrow widths and renders multiline input with `↵`.

The already-resolved unicode-segmentation 1.13.3 and unicode-width 0.2.2 dependencies
are now declared directly; no new versions were downloaded. Four editor regressions
cover middle edits, combined Unicode, viewport width bounds, bounded paste, word
removal, submission reset and independent session cursors. The PTY journey edits
`fst` into `fast` with actual cursor keys while another model request is stalled.
Final verification: 58 Rust tests passed, three live/subprocess fixtures ignored;
formatting, strict Clippy, locked release and release PTY passed (0.820 s).
Documentation validation: A (98.2%). Log: `/tmp/alfredo-editor-tests.log`.

## Model discovery continuation

`/models` now retrieves the configured Ollama server's installed-model catalog
asynchronously, validates names, sorts/deduplicates them, and displays a scrollable
catalog. `/model NAME` explicitly selects a listed model for the current conversation.
Active/interrupted turns keep their original model; selecting for another conversation
never changes existing task assignments. New task proposals inherit their originating
conversation model. Escape closes the catalog without cancelling inference.

Discovery uses a ten-second total deadline and bounds response bytes to 1 MiB and
entries to 256. Failed discovery retains the last known catalog with a visible error;
a listed name does not prove loaded-model readiness. Protocol reference:
[Ollama list models](https://docs.ollama.com/api/tags).

New regressions exercise real GET transport, malformed/control/oversized responses,
sorting/deduplication, explicit selection and active/interrupted model continuity.
The release PTY journey discovers and selects a second model while another model
request remains stalled, then completes the selected conversation independently.
Final verification: 61 Rust tests passed, three ignored live/subprocess fixtures;
formatting, strict Clippy, locked release build and release PTY passed (0.861 s).
Log: `/tmp/alfredo-models-tests.log`.

## Durable conversation continuation

The terminal now restores named conversation sets within each workspace/mission.
Schema-v1 snapshots retain models, transcripts, drafts, cursor positions, selected
conversation, attempt identity and status. Saved Connecting/Streaming states restore
as interrupted with partial output preserved, and never launch inference on reload.
Normal shutdown cancels active client requests and persists the terminal snapshot.

A separate OS owner lock protects each named set; a second terminal can share the
task queue by choosing another `--conversation NAME`. Autosave runs about once a
second off the input thread with only one write in flight and current-state
coalescing. Shutdown joins the old write before final publication. Files use private
permissions, bounded reads, exclusive temporary creation, sync and atomic replacement.
Unknown versions, role/lifecycle/cursor corruption and symlink targets fail without
silently resetting history. Abrupt termination can lose the newest changes since
the last completed checkpoint; this is not a zero-loss journal. Conversation content
is separate from authoritative task receipts. No legacy Python history is imported.

Six regressions cover restored drafts/cursors/models/selection/partial text, explicit
retry after interruption, owner exclusion and namespace isolation, future/malformed
state, final-save ordering, invalid-write preservation and symlink rejection. The
release PTY closes and reopens real conversations and verifies the second model and
completed response before continuing the durable coding task. Final gate results: 67 Rust tests passed, three live/subprocess fixtures ignored;
formatting, strict Clippy, locked release and release PTY passed (0.869 s).
Log: `/tmp/alfredo-conversation-tests.log`.

## Prompt history and command discovery continuation

Conversation Up/Down now browses bounded history and returns to the original unsent
draft and cursor. Task-view arrows keep their task-selection behavior. History is
limited to 100 entries/128 KiB per session and suppresses adjacent duplicates.
Saved user prompts seed restored history; transient slash-command history is not
persisted. Autosave captures the original unsent draft while history is being
browsed, avoiding loss of the user's unfinished text on restart.

F1 with an empty draft opens the command picker. Tab after a slash prefix opens
matching commands; Up/Down/Tab selects, Enter fills the draft, and Escape dismisses.
Completion never submits a task or model request. A second Enter goes through the
existing command/approval path. Four regressions cover Unicode draft restoration,
command filtering/cycling/rendering without effects, autosave during history browsing,
and bounded isolated histories. The real PTY journey completes `/mod` into `/models`
while another inference remains stalled, then explicitly submits it. Issue #61
still lacks @-capability completion and the full capability-attributed Mission
formation journey. Full verification: 71 Rust tests passed, three live/subprocess
fixtures ignored; formatting, strict Clippy, release build and release PTY passed
(0.874 s). Final history-bound focused tests/Clippy/rebuild also pass. Documentation
validation A (98.2%). Log: `/tmp/alfredo-command-tests.log`.

## Saved task activity continuation

`/activity` renders acknowledged task receipts newest-first, with text search or
exact `#ID` filtering. Each entry shows revision, task, action summary/detail and
correlation identity. Proposals, policy changes, approvals, run claims, terminal
results and accept/reject decisions remain distinct. Task titles and receipt IDs
are searchable. This is a read-only projection of the existing receipt ledger,
not a second store, and it invents neither timestamps nor actor identities.

Regression coverage verifies ordered history, exact replay without duplicate
activity, exact task filtering (excluding task #10 from #1), case-insensitive search,
absence of rejected requests, readable wide/narrow layouts and byte-identical store
state after navigation. The release PTY reads restored approval activity and later
finds the saved review acceptance. The full attributed Activity Journal and unified
conversation/action chronology remain incomplete. Final verification: 72 Rust tests
passed, three live/subprocess fixtures ignored; formatting, strict Clippy, locked
release and release PTY passed (0.871 s). Log: `/tmp/alfredo-activity-tests.log`.

## Shared inference admission

Chats and coding workers share a client inference limit of two active requests per
terminal. `--parallel-models N` sets that limit from 1 to 8. Waiting requests show
“Queued for model slot” and can be cancelled before HTTP dispatch. Model discovery
bypasses this queue. The ten-minute total request deadline includes queue time;
the loading/idle deadline starts after admission. This limit does not coordinate
separate terminals or prove server-side cancellation of an active request.

Two regressions verify real HTTP admission across ordinary/structured provider
clones, cancellation without dispatch, release to the next request, and stale
queue events after cancellation. Full Rust suite: 74 passed, three ignored
live/subprocess fixtures (`/tmp/alfredo-admission-tests.log`). Formatting, strict
Clippy, locked release build and release PTY pass (0.871 s; session 91411 terminal).
No latency improvement or complete Issue #69 parity is claimed.

## Receipt-linked repair workflow

`/repair ID reason` now proposes a new child of a failed, rejected or cancelled
run with retained verified evidence. It inherits model, exact file/check policy
and dependencies, requires fresh approval, and prevents a second unresolved child.
Exact proposal replay returns the original child. The worker revalidates parent
evidence (128 KiB context cap), pins the parent baseline even if workspace HEAD
has moved, and passes prior task/patch/check outcome as reference data for complete
corrected file generation. Parent worktree/evidence/state remain unchanged.

Schema v3 adds optional repair lineage and Repair receipts. Schemas v1/v2 retain
exact-byte backups and acquire no inferred permissions or lineage. Tests exercise
both upgrades, rejected-parent repair, replay, duplicate/approval guards, exact
baseline enforcement, real model request context, a sandboxed repair check and
evidence tampering. Full suite: 75 passed/three ignored; final focused test and
strict Clippy pass. Locked release build passes. Initial PTY failure expected a
nonexistent notice; corrected visible task-state assertion passes actual terminal
reject/repair/approve/run/accept (1.102 s, session 39410). Original workspace remains
unchanged by workers. Logs: `/tmp/alfredo-repair-tests.log` and
`/tmp/alfredo-repair-focused.log`. Automatic repair routing/escalation and uncertain
effect reconciliation remain incomplete.

## Live check output

Selected tasks now render transient stdout/stderr tails while an approved check is
running. Each stream retains at most 8 KiB. Shared Rust execution has an optional
advisory callback fed by a nonblocking 32-chunk queue with 4-KiB chunks. Slow
consumers can miss chunks; capture threads continue to enforce the original
aggregate output limit and preserve receipt bytes. The terminal filters control
characters before rendering. Finished workers discard live tails and expose saved
output through verified evidence. Desktop callers supply no output observer and
retain their JSONL protocol.

A real sandboxed check prints to both streams and waits; the regression observes
output while the durable task is Running, renders it, cancels, and verifies the
saved Cancelled result retains output. Another regression leaves the observation
queue unread and proves capture completes with exact bounded bytes and output-limit
signalling. Full terminal suite: 76 passed/three ignored, plus the later focused
backpressure test (77 distinct passing tests). Strict Clippy, terminal/desktop
formatting, locked release and release PTY pass (1.160 s). Shared compatibility:
legacy Rust 65 passed/one ignored; focused Python 83 ran/one skip in 10.235 s.
Logs: `/tmp/alfredo-live-output-tests.log`,
`/tmp/alfredo-live-output-legacy-rust.log`, `/tmp/alfredo-live-output-python.log`.
Sessions 92564, 97954, 13162 and 80649 completed. No schema migration or new task
authority follows from live output.

## Native Linux development archive

`alfredo-tui/scripts/package_release.py` now requires Rust 1.96.0, explicitly
builds its qualified native x86_64-unknown-linux-gnu target with Cargo.lock and
packages the executable, standalone INSTALL.md, lockfile and BUILD.json. Provenance
records compiler, source fingerprints, Git revision, dirty status and payload
checksums; a companion SHA-256 verifies the archive. Output directories must be
new. Fixed archive metadata/order makes identical input packaging deterministic.
The script does not publish, install host dependencies or mutate runtime state.

`tests/release_smoke.py` verifies archive and member checksums before installing
the executable into a temporary directory containing spaces. CLI help/version and
the full Linux PTY conversation/model restart, isolated coding, reject/repair,
approve/run/accept journey run outside the checkout with a restricted PATH. A
corrupted candidate is rejected before execution. Final installed PTY passed in
1.070 s (session 81649 completed). Final C/D candidates are byte-identical.

Artifact: `/tmp/alfredo-terminal-candidate-c/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`
(2,435,874 bytes). Archive SHA-256:
`ac722dd6cb9d53e3b890ed1d958838a5e5882aa2916db72575bea3de902d5ee4`.
Binary SHA-256: `fcc100206611737d3c416141aa9be021ed5be09320bb978adf55d4ab24dbfe81`.
The manifest truthfully records uncommitted source. These are development
candidates, not a public release or cross-host reproducible-build proof.

The Rust terminal CI workflow now packages, verifies installation and retains the
candidate using upload-artifact after its existing gates. It has not run remotely.
Other operating systems/architectures, older glibc, project/dependency license
review, complete product acceptance and publication remain open. This does not
close the desktop npm requirement in Issue #11 or replace its distribution.

## Readable evidence and exact review target

Verified evidence is now parsed once into a check summary, colored unified diff
with preserved `+`/`-` markers, and separate saved stdout/stderr. The view names
its task/run/baseline and shows missing or uncertain check outcomes explicitly.
Raw output cannot establish check success. Control characters are removed before
rendering. Scrolling clamps to content; narrow evidence review uses the task-list
area so 32x10 retains visible content.

A real PTY reproduction opened `/evidence 2` while task 1 remained selected, then
`/accept` targeted the rejected task 1 and failed. Selection now follows verified
evidence acknowledgement. The updated PTY accepts task 2 using shorthand and
asserts task 1 remains Rejected. A first layout regression failed because the
stacked list left no evidence content rows at 32x10; the compact review layout
fix passes. The store still verifies the digest before display and again on review.

Final full terminal suite: 79 passed/three ignored
(`/tmp/alfredo-review-final-tests.log`). Formatting, strict Clippy, locked release
and real terminal journey pass (1.144 s; session 4367 completed). New renderer
regressions cover signed/color diff lines, Unicode/control stripping, missing
check truth, malformed data and scrolling after resize. Initial failed layout log:
`/tmp/alfredo-review-tests.log`. No shared execution/legacy authority code changed.
The earlier candidate C predates these review fixes; use the refreshed candidate
at `/tmp/alfredo-terminal-candidate-review/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`. Installed acceptance passes (1.068 s; session 37920 completed). Archive SHA-256: `221cb7aa25d4f78c7f4796e361667f14ebc8431c4d6dbcd1f5515a4d27aba564`.

## Noninteractive startup diagnostics

`--doctor` now checks startup without raw terminal mode. It uses the same task
and named-conversation store validation as startup, chooses the saved selected
model when present, and queries the bounded Ollama catalog without inference.
Sanitized bounded Git commands inspect the root, commit and rejected checkout
configuration; metadata checks locate Git, Bubblewrap and prlimit. Diagnostics
report storage/server/catalog/worker failures together with actionable flags and
exit 2. Exit 0 describes these preflight checks, not GPU capacity, actual sandbox
permissions or model speed. It may initialize private lock namespaces; it does
not save conversations or mutate task receipts.

Actual CLI regressions verify restored-model precedence, successful and missing
catalog entries, no terminal escapes, preservation of saved task/conversation
bytes, and combined unreachable-server/invalid-storage failure without erasing
the original file. Full Rust suite: 81 passed/three ignored
(`/tmp/alfredo-doctor-tests.log`). Formatting, strict Clippy, locked release and
release PTY pass (1.134 s, session 3702 completed). The installed-candidate PTY
journey now runs diagnostics against its HTTP fixture before entering the terminal.
Installed acceptance passed (1.120 s; session 82264 completed). Current candidate:
`/tmp/alfredo-terminal-candidate-doctor/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`; SHA-256 `fdde3c0d1e668cc561bb25a03aa1c03df3a6c9337ffa671eab064049ca09621d`.

## Candidate snapshots for dependency composition

Successful workers now stage only their tracked/intent-to-add content and create
an immutable Git commit parented by the recorded baseline. The exact binary diff
is compared with saved evidence before retaining a `refs/alfredo/candidates/<hash>`
ref. Workspace branch, HEAD and working files are unchanged. Untracked check/build
output is excluded. Hooks and signing are disabled; Git replacement objects are
ignored during exact-object reads. Failure to capture a matching candidate prevents
new successful-check work from becoming review-ready.

Evidence gains an optional candidate commit ID under its existing digest. Legacy
artifacts stay readable with no inferred candidate, and no task-store schema or
approval upgrade occurs. The public verifier checks exact parent/diff without
relying on mutable retained worktree files. New tests assert source HEAD/status
preservation, new-file inclusion, managed-ref retention, verification after
worktree mutation, mismatched patch/parent rejection, and refusal to infer a legacy
candidate. The final focused test also configures signing with an unavailable
program and proves internal candidate creation does not invoke it.

Full Rust suite: 82 passed/three ignored (`/tmp/alfredo-candidate-tests.log`).
Initial formatting, strict Clippy, locked release and release PTY pass (1.147 s).
Final signing-disabled focused test passes
(`/tmp/alfredo-candidate-signing-test.log`); final Clippy, locked release and PTY pass (1.144 s; session 48987 completed).
No verification processes remain pending. Current installed
candidate doctor predates this source change. Candidate snapshots are a prerequisite,
not completion of dependency execution: accepted-parent composition, conflict
handling and verification at dispatch remain next. Candidate-ref retirement and
storage budgeting remain open.

## Accepted dependency execution

Dependent `/run` now verifies every accepted parent's retained candidate against
saved evidence, then combines the candidates with current committed HEAD using
Git merge-tree. Ancestor inputs are reused, including diamonds; conflict output
names affected paths and leaves the child unstarted. Custom merge configuration
is refused before invoking a merge, so configured external drivers cannot run.
Preparation observes cancellation and a 60-second deadline. It may create derived
Git objects and `refs/alfredo/bases` before the run claim; it never moves a branch,
changes workspace files, starts inference or invokes the approved check at that
stage. The child worktree/check starts only after a successful durable run claim.

Schema v4 adds exact dependency input task/run/evidence-digest/candidate records
to Start receipts and TaskRun. Transactions recheck saved input evidence; replay
validates accepted identities. Versions 1–3 retain exact-byte backups and gain no
inferred inputs. The child's review diff is relative to its composed baseline,
and the UI lists acknowledged dependency inputs. Workspace branch publication,
automatic conflict repair and object/ref retirement remain unfinished.

Real tests cover two independent parents, a child check that reads both results,
diamond ancestry, unchanged source HEAD/working files, unaccepted/conflicting/
tampered parents, and refusal of a configured external merge driver. Full suite:
84 passed/three ignored (`/tmp/alfredo-dependencies-final-tests.log`). Formatting,
strict Clippy, locked release and dependent-task PTY pass (1.468 s; session 1086).
The first PTY ran a binary built before the provenance view update; final rebuild
passes. That output also exposed underlying text showing through overlays; review,
activity and model overlays now clear their area before rendering.
Final focused tests also verify exact backup of an accepted v3 run and unchanged
source HEAD (`/tmp/alfredo-dependencies-migration.log`). Final strict Clippy and
installed acceptance pass (1.418 s; session 36582 completed). Candidate:
/tmp/alfredo-terminal-candidate-dependencies/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz
SHA-256: `933694f2e01d3efbbc16e911c448098c04686b4117aee4640ca1b636239e4e0b`. No verification processes remain pending.

## Accepted-result local branch handoff

`/branch [ID]` now requires Accepted work and verifies its candidate parent/diff
against saved evidence. It creates `alfredo/task-ID-COMMITPREFIX` from an absent
ref using Git compare-and-set, or confirms an exact existing ref. Different targets
and symbolic refs are refused. It never checks out, changes HEAD/index/working
files or pushes. A task Branch receipt records the verified local name/commit;
activity and details expose the historical handoff and the notice prints a safe,
deterministic `git switch` command for the user's next action.

If Git succeeded but receipt storage was interrupted, repeating the command
verifies the existing ref and reconciles the receipt. Concurrent identical requests
also produce one branch and one receipt; the losing creation observes the exact
ref before continuing. Older receipts do not claim present-day immutability of a
branch: a later external move is detected by another `/branch` call and preserved.
Schema v5 adds Branch receipts only; versions 1–4 get exact-byte backups with no
inferred handoffs. Errors offer generic task retry only when a task request is
actually available, avoiding an unrelated retry hint for branch errors.

Full Rust suite: 86 passed/three ignored (`/tmp/alfredo-branch-tests.log`).
Formatting, strict Clippy, locked release and complete branch-handoff PTY pass
(1.464 s; session 59514 completed). Final focused concurrency/collision/migration
coverage passes (`/tmp/alfredo-branch-concurrency.log`) and final strict Clippy
passes (session 98718 completed). Tests verify dirty tracked/untracked workspace
preservation, unchanged HEAD, exact branch file contents, unaccepted refusal,
replay, conflicting and dangling-symbolic-ref refusal, and v4 accepted-result
reconciliation with an exact-byte backup. All Git effects were in temporary test
repositories; no source-checkout review branch or remote publication was created.
Installed acceptance passes (1.392 s; session 78053 completed). Candidate:
`/tmp/alfredo-terminal-candidate-branch/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `00658ed6a5b2109e27d0a18660852b7a62cfcd997d0c2003f23dc234dada6c1f`.
No verification processes remain pending.

## Worker completion capacity checkpoint

The receipt-limit regression reproduced an unrelated proposal consuming the last
journal slot while a worker remained Running. A replay-valid near-4-MiB fixture
also reproduced a command consuming the space needed for an escaped result.
Non-Finish transactions now project worst-case completions for every Running task
under the transaction lock before saving. The projection accounts for JSON escaping,
receipt identities, both copies of result details, status and revision growth.
New claims reserve their own completion; Finish consumes its reservation while
remaining subject to actual hard limits. No schema change or history deletion occurs.

Three public-store regressions cover receipt exhaustion, escaped byte exhaustion,
exact Finish replay and a rejected second run claim that leaves stored bytes intact.
Full Rust suite: 89 passed/three ignored (`/tmp/alfredo-capacity-tests.log`). Formatting,
strict Clippy and locked release pass. Release PTY passes in 1.475 s (session 38027
completed). Documentation validation remains A (98.2%). Historical overcommitted
journals may still lack space for an actual result; archive/capacity reclamation
and Git-ref/storage cross-system handoff atomicity are not supplied by this fix.

Installed archive acceptance passes in 1.406 s (session 48341 completed):
`/tmp/alfredo-terminal-candidate-capacity/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `b0887f6e669e8f061734a6d1272815cd0ddd22d2bebaf7a5e977ae7868c7c3d4`.
No verification processes remain pending.

## Branch admission checkpoint

Regression fixtures reproduced branch creation before rejection of an invalid
correlation or exhausted receipt journal. Branch handoff now asks TaskStore for
read-only admission before Git mutation. It validates correlation conflicts, current
revision and the prospective schema-v5 receipt against count, serialized byte and
running-worker completion reservations. Existing exact Branch receipts require no
new space, so replay still works at full capacity. This check writes no task state,
backup or branch and is not an atomic reservation across Git and storage.

Three public branch regressions cover invalid identity, 4096-receipt exhaustion,
replay using the final slot and near-4-MiB escaped-text exhaustion. Rejections leave
both the journal and absent ref unchanged. Existing concurrency, dirty workspace,
collision, symbolic-ref and v4 reconciliation regressions remain passing. The final
receipt transaction still handles intervening mutations; exact-ref reconciliation
remains necessary for races or subsequent disk failures.

Full Rust suite: 92 passed/three ignored (`/tmp/alfredo-branch-preflight-tests.log`).
Formatting, strict Clippy, locked release and release PTY pass (1.455 s; session
61776 completed). Installed acceptance passes (1.407 s; session 97578 completed):
`/tmp/alfredo-terminal-candidate-preflight/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `80f37bb996c101a7ae3210405c744baa1385e2744fe270e4ec6fe3f949c32dff`.
Documentation validation A (98.2%); no verification processes remain pending.
No GitHub publication was performed.

## Prompt-to-plan checkpoint

Live GitHub #59, #35 and parent #56 were read before implementation. The current
Rust rewrite request authorizes the native shell while preserving their proposal /
approval / launch distinctions. `/plan REQUEST` uses the shared bounded Ollama client
and strict structured output to generate 1–16 ordered Local Agent task drafts.
The selected model acts as Frontier Architect and is the explicit worker model for
this initial path. The preview shows goals, exact file/check policy and dependencies;
it is labelled unsaved/unapproved and does not inspect repository files.

`/plan-save` records one schema-v6 Plan receipt containing the original prompt,
planner and full batch. Replay expands all steps to Proposed tasks, rebasing local
step dependencies to durable IDs. All validation/capacity checks and one atomic save
happen under the task lock; no partial batch, approval or run can result. Versions
1–5 upgrade with exact-byte backups and cannot carry Plan receipts. A stale revision
refuses publication and retains the draft. Exact receipt replay does not duplicate
steps; activity queries for any child find the plan receipt.

`/plan` reopens the transient draft and `/plan-cancel` aborts inference and discards
pending results. Strict schema parsing, model-identity checks and graph/policy bounds
prevent incomplete, malformed or invalid model output becoming task authority.
The preview remains labelled untrusted until explicitly saved. Six new integration
regressions cover atomic/rebased/replay publication, refusal without approval,
malformed/incomplete output, stale saves, schema downgrade rejection, cancellation
of an unobserved result, and wide/32x10 draft rendering with hide/reopen navigation.

The initial release PTY exposed an existing fixture race: “Accepted” also appears
in explanatory evidence text. The wait now targets the exact review receipt revision,
without weakening task-state assertions. Final release PTY passes in 1.852 s
(session 98773 completed), including no mutation during preview and one saved Plan
receipt with two dependent Proposed tasks. Strict Clippy and formatting pass.
Repository-aware planning, worker model/profile overrides, durable draft continuity,
Shared Understanding / Plan Grill gates and automatic graph dispatch remain open.

Full final suite: 98 passed/three ignored (`/tmp/alfredo-plan-final-tests.log`).
Installed archive acceptance passes in 1.743 s (session 98912 completed):
`/tmp/alfredo-terminal-candidate-plan/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `976c7cecc98fd9d7b9f2bb63fd21553b000954bf84c176918daaceb4823f7392`.
Documentation A (98.2%); no verification processes remain pending. No GitHub
publication was performed.

## Worker model assignment checkpoint

`/assign ID MODEL` checks the current installed-model catalog before recording a
new unstarted worker assignment. TaskStore owns lifecycle validation: only Proposed
or Approved tasks without a run may change models, preserving policy/dependencies
and returning to Proposed for fresh approval. The model of a started or terminal
run cannot change. Plan receipts retain their original architect/model provenance;
a subsequent Assign receipt explains the task's current worker model.

Catalog lookup holds no task lock. Publication rechecks the expected revision, so
concurrent cancellation or other state changes refuse the request. Generic task
retry uses the same assignment adapter and cannot bypass a missing-model refusal.
An exact already-acknowledged request replays without a model server; changed
correlation payloads refuse. The notice distinguishes the saved assignment from
the task's current model/state. Schema v7 adds Assign receipts; all six older
schemas upgrade with exact-byte backups and cannot fabricate assignment authority.

Three integration regressions cover policy preservation, approval reset, premature
run refusal, offline replay after reopening, changed correlation, immutable started
runs, state changes during catalog lookup, retry admission and legacy rejection.
The expanded PTY journey approves a generated task, reassigns it to the other
installed model, observes Proposed state, sets an exact check and approves/runs it.
It verifies the actual model HTTP request, saved assignment and unchanged source file.
Catalog membership is not qualified model/profile availability or proof of quality.
Native Issue Slice assignment, configured agent/profile registry and qualified role
selection remain open, along with repository-aware planning and automatic dispatch.

Full Rust suite: 101 passed/three ignored (`/tmp/alfredo-assignment-tests.log`).
Formatting, strict Clippy, locked release and release PTY pass (2.226 s; session
75223 completed). Installed archive acceptance passes (2.140 s; session 85358
completed): `/tmp/alfredo-terminal-candidate-assignment/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `07beb71f71b807c87998f6d081425c38ad22be912d4f507f085614661f6b9ac9`.
Documentation A (98.2%); no verification processes remain pending. No GitHub
publication was performed.

## Committed repository planning context checkpoint

`/plan` now captures a pinned committed Git tree and a bounded selection of UTF-8
source blobs before model dispatch. It uses sanitized read-only Git commands and
never follows working-tree symlinks or reads uncommitted/untracked files. Root
instructions, README/context and build manifests rank ahead of request-matching
paths. The preview exposes the commit, selected source names and omission counts;
the exact context is supplied as reference data before the user request.

Bounds are explicit: the Git helper limits output to 256 KiB; context capture has a
30-second total deadline; the file map holds at most 256 paths / 16 KiB; at most
32 candidate blobs are considered for eight complete UTF-8 files, 8 KiB each and
24 KiB combined, with a 64-KiB serialized context ceiling. Symlinks/gitlinks,
generated/vendor paths, common secret-file names, oversized and binary sources
are omitted. This is neither full project inspection nor comprehensive secret
classification. Missing/invalid context fails before inference. Cancellation drops
both context collection and subsequent model futures.

Schema v8 retains exact baseline/path-map/source-path/blob/content and omission
counts in the Plan receipt. Earlier schemas gain no inferred inspection; context
in a v1–v7 receipt is refused. Workers verify the original plan baseline before
preparing dependency inputs and claiming a run, so changed committed HEAD cannot
silently replace the source the plan used. Accepted dependencies still compose
onto the pinned baseline. Historical ungrounded plans retain their earlier behavior.

Three new regressions cover bounded selection with dirty/untracked/secret/symlink/
binary/oversized exclusions, saved-context replay with changed-baseline run refusal,
and context failure without any model request. Existing planner fixtures verify
committed README content reaches the actual HTTP payload. A repeated-keyword test
reproduced source names outranking AGENTS.md; request scoring is now capped so root
instructions retain priority. The initial full suite and release PTY passed
(2.268 s; session 52515 completed). The final full suite passes 104 tests with three ignored fixtures/live tests
(`/tmp/alfredo-context-final-tests.log`); formatting and strict Clippy pass. Final
installed archive acceptance passes 2.139 s (session 91254 completed):
`/tmp/alfredo-terminal-candidate-context/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `517fb231ddd6a802301f3f1884330f63ae84d36c0503e6a26a192fa37f15dea0`.
Documentation A (98.2%); no verification processes remain pending. Dynamic retrieval, comprehensive project instruction coverage,
uncommitted/new-repository planning and context-budget qualification remain open.

## Ready-task dispatch checkpoint

`/dispatch on|off` adds deterministic process-local dispatch through the same worker
path as `/run`. It selects Approved tasks with exact policy, no run and Accepted
dependencies, shares the four-worker/model limits and waits for an acknowledged
claim before selecting another global revision. It records each automatic/manual
start attempt by approval revision in memory; failures do not loop. Explicit /run
can retry, and a new approval permits another automatic attempt. Pending or uncertain
runs remain governed by existing receipt/recovery checks. Dispatch starts off in
every process and is disabled before shutdown cancellation; no schema change or
automatic approval/review is introduced.

Background refresh uses a separate bounded channel so it cannot reject a user command
as “pending” or accidentally clear a write receipt. Snapshot revision monotonicity
and the current action notice survive delayed refresh. Read failure disables dispatch
visibly. Selected task details retain start errors even when another worker completes.
Three store/controller regressions cover dependency/approval selection, once-per-
approval attempts, failed manual-start suppression, restart-off behavior and command
admission during background refresh.

The real PTY fixture requires two independent workers to reach inference concurrently
using a two-party server barrier. It also keeps an approved child unstarted until
its parent is Accepted, then verifies one run receipt per task and bound dependency
inputs. Initial acceptance reached the child but exposed an unsuitable generated
fixture check: unittest with no tests returned exit 5. Evidence confirmed dispatch
had correctly waited and launched once. The fixture now explicitly permits and
approves an inherited-value assertion. Worker failure notices include the check exit
code; the authoritative provider check outcome remains unchanged.

Cross-process scheduler/lease coordination, durable dispatch intent, automatic
resolution of preclaim revision contention and the broader launch gates remain open.

Final full Rust suite: 107 passed/three ignored (`/tmp/alfredo-dispatch-final-tests.log`).
Strict Clippy and release PTY pass (3.646 s; session 73782 completed). Quit disables
dispatch even when a foreground save is pending; CLI help lists the new commands.
Final CLI tests and Clippy pass. Installed archive acceptance passes (3.528 s;
session 8561 completed): `/tmp/alfredo-terminal-candidate-dispatch/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `f4a64e4c5187aa8f94a2991ffccc31ca32937eb100b01728f19d349a8567be7d`.
Documentation A (98.2%); no verification processes remain pending. No GitHub
publication was performed.

## Task supervision and search checkpoint

`/tasks QUERY` searches task title, model, status and readiness text; `#ID` targets
an exact task ID and `/tasks` clears the query. Search is bounded to 200 bytes,
transient and read-only. Task selection and shorthand commands use visible rows
only. An empty match set has no implicit target. Evidence acknowledgment clears
the filter and selects the exact inspected task, so hidden-task review cannot
accidentally act on the previously selected visible row.

The sidebar separates ID/status from title and shows visible/total counts and the
filter. Details distinguish missing policy, missing approval and exact unaccepted
parents, including ReviewReady parents that have not yet been Accepted. These are
explanations, not replacement runtime authorization. Dispatch remains global and
the UI says so. Text/status matches avoid unnecessary blocker construction, and
blocker lookup uses the validated contiguous task identity layout rather than
repeated full scans.

Two new regressions cover filtered navigation, exact shorthand effects, read-only
search, safe empty matches, query bounds, visible blocker explanations and the
policy/approval/dependency distinctions. The full Rust suite passes 109 tests with
three ignored fixtures/live tests. Initial strict Clippy and release PTY pass
(3.605 s; session 66623 completed). The PTY deliberately filters to task #1 before
opening evidence for hidden #2 and confirms shorthand acceptance still targets #2.
Final focused task/layout/review/CLI tests and Clippy pass after the sidebar-count
and lookup refinement. Installed acceptance passes in 3.552 s (session 54802 completed):
`/tmp/alfredo-terminal-candidate-task-view/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`, SHA-256 `8402c607e3f5e7b79d9371f220439a47df5c6e532df2689938356fe4b5c29b3c`.
Documentation validation remains A (98.2%); no verification processes remain pending. Native Issue Graph hierarchy, view-preference continuity and
human accessibility acceptance remain open.

## Server timing and live loading diagnosis

Optional final-frame Ollama metrics now appear in conversations, Frontier Architect
previews and verified Local Agent evidence. Invalid auxiliary fields are ignored
independently; only bounded unsigned durations/counts are admitted. Zero generation
duration never produces a rate. Metrics cannot substitute for completion, checks,
approval or review. Conversation/plan observations remain transient; optional worker
metadata is covered by existing evidence digests and older evidence stays readable.
Protocol: [Ollama chat response](https://docs.ollama.com/api/chat).

Live qwen2.5-coder:14b, same prompt “Reply with only the word READY.”:

| Observation | Client first content | Client completion | Server load | Server prompt | Server generation |
| --- | ---: | ---: | ---: | ---: | ---: |
| Initial | 18.912 s | 18.948 s | 18.71 s | 0.20 s | 0.04 s |
| Repeat | 0.412 s | 0.438 s | 0.14 s | 0.27 s | 0.03 s |

Both returned READY. Logs: `/tmp/alfredo-metrics-live.log` and
`/tmp/alfredo-metrics-live-warm.log`. Loading dominated the initial request; these
are two observations of a tiny prompt, not a sustained coding benchmark or proof
of a performance improvement caused by this change. Residency policy is unchanged.

Verification: full Rust suite 111 passed/three ignored; strict Clippy and release
PTY pass (3.752 s). Installed archive acceptance passes (3.562 s):
`/tmp/alfredo-terminal-candidate-metrics/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`,
SHA-256 `f18ed475b4acaac23e60891197207dc7699440a5bf21f384293da42d8e47e936`. Sessions 22008 and 82333 completed. Documentation A (98.2%);
no GitHub publication occurred. Provider regressions cover malformed/absent/bounded
metrics, zero-rate handling, event ordering and stale/restarted attempts. Worker
coverage asserts saved values; the real PTY asserts rendered server timing using
synthetic fixture metrics. Full launch acceptance remains open.

## Task view restart continuity

Conversation schema v2 retains tasks/chat mode, selected task ID and bounded search
within each existing workspace/mission/named conversation set. Old v1 state restores
without inferred preferences and is backed up byte-for-byte before replacement.
Malformed state and differing existing backups fail unchanged. Selection resolves
against the refreshed visible task list; absent matches select nothing. Dispatch
starts off and evidence must be verified again. Task schema remains v8.

Full Rust suite: 113 passed/three ignored; strict Clippy passes. Initial terminal
fixture assumed chat on restart, contradicting the newly saved Tasks view; it now
asserts that restored view and explicitly switches to chat to verify saved replies.
Final release PTY passes in 3.729 s; installed archive passes in 3.571 s. Tests cover
migration backups/conflicts, invalid view metadata, no task effects from restored
selection, and a real restart restoring filter #7 without journal changes.

Archive: `/tmp/alfredo-terminal-candidate-view-continuity/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256: `c1e6f0ce5a8f6d067e355ed0001aa2f8b1c3fd19dbdeb1b4ba6c30a4c7097cf7`. Sessions 40486, 49679 and 11592 are complete; no verification
processes remain pending. Documentation A (98.2%). No GitHub publication occurred.
Interactive workspace/mission selection, planning-draft continuity and the rest of
the production launch requirements remain open.

## Selection-required native startup

Startup now keeps the shell Starting Location separate from an acknowledged Coding
Workspace. The terminal validates an exact Git root or explicitly creates a new
unused repository, then requests a mission name. Both CLI selections share root
validation and skip forms; doctor remains noninteractive. No conversation/task state
or model request starts before normal interactive selection completes. Validation
runs asynchronously, with terminal-safe status text and grapheme-aware path editing.

New creation refuses existing targets, nested repositories and runtime overlap,
uses sanitized Git init with an empty template, and retains partial failures without
automatic replay. It creates no initial commit, so a new project needs a commit
before repository-grounded planning or workers. This is startup selection, not the
complete Workspace Session/mission-formation receipt model from the desktop.

Verification: 115 Rust tests passed/three ignored; strict Clippy and release PTY
passed (3.700 s). Git fixtures exercise creation, exact-root validation, Unicode and
space paths, existing-file preservation and runtime separation. Actual PTY traverses
workspace and mission steps before conversations/multiple workers, verifies no early
conversation snapshot/inference, and uses explicit selections for restart. Final
control-text sanitization passes Clippy and installed archive acceptance (3.785 s).

Archive: `/tmp/alfredo-terminal-candidate-selection/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256: `71b99e1ede9379c3cb4f705ba5d15bbf7fd97e68c4f926639dc3455bda9e7317`. Sessions 68001/98494 completed; no verification processes pending.
Documentation A (98.2%); no GitHub publication. Recent-work discovery, in-terminal
switching, durable Workspace Session receipts and mission formation remain open.

## New repository preflight and Create-mode acceptance

A failing regression demonstrated runtime overlap hidden behind a symlink alias:
selection returned an error only after the proposed repository had been initialized.
The new preflight resolves existing runtime ancestors and appends the future suffix
before comparing paths. Overlap, dangling runtime symlinks, invalid ancestors and
unresolvable parents now refuse before create-dir/Git init. Later validation remains;
there is no cross-process filesystem atomicity claim.

Red evidence: `/tmp/alfredo-selection-alias-red.log`, failed assertion “Rejected
selection created repository files”. Green selection suite: four passed, including
the alias fixture and dangling/non-directory runtime ancestry. Full suite 117 passed,
three ignored; strict Clippy and release PTY pass (3.817 s; session 35690 completed).
The real terminal now exercises F2 Create mode, refuses an existing target, creates
an empty Git repository at a path containing spaces, enters a mission and exits with
terminal modes restored. It checks no conversation state before mission selection,
no fabricated initial commit and no changes to the previous workspace task journal.

Installed archive acceptance also passes (3.651 s; session 76912 completed):
`/tmp/alfredo-terminal-candidate-creation-preflight/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`. SHA-256 `39547184540b95250532109ab4bbdee489c745747ef0c02b9e60eb0df7efe7f7`.
No verification processes remain pending; documentation A (98.2%), diff check clean.
No GitHub publication occurred. Mission formation and full launch acceptance remain open.

## Saved-mission discovery and recovery stability finding

Startup asynchronously discovers names for the selected repository from immutable
advisory identity sidecars and legacy task-journal identities. Tab fills a name;
Enter explicitly opens it. Discovery does not establish task status or approval,
and normal loading retains receipt validation. Conversation-only missions registered
on opening become discoverable; older such namespaces require one manual named opening.

Bounds: 1,024 namespace entries, 16 MiB aggregate reads, 8 KiB identity records and
4 MiB legacy journal probes. Invalid/misplaced/symlink records are skipped, limits
are visible and manual names remain available. Atomic create-only identity publication
preserves conflicting files. Tests cover conversation-only/legacy discovery, workspace
isolation, non-mutating reads, corrupt identities, entry/byte bounds and state symlinks.

Initial full suite failed the existing `missing_truncated_or_false_success_evidence_never_changes_claim`
assertion in recovery.rs without printing its error. The assertion now includes error
detail without changing its expectation. A focused recovery run and 100 diagnostic
native recovery-suite repetitions passed; the final full rerun passed. The original
error is not reproduced and its cause remains unconfirmed. An inherited file-lock
window is a hypothesis only; worker ownership logic was not changed. Keep this as
an open stability finding rather than claiming it fixed. Logs:
`/tmp/alfredo-missions-full.log`, `/tmp/alfredo-missions-recovery-diagnostic.log`,
`/tmp/alfredo-missions-final-full.log`.

Final Rust suite: 122 passed/three ignored; strict Clippy and formatting pass.
The new-mission PTY initially timed out on quit. Diagnostic screen capture exposed
stale primary-screen text in the test emulator, which ignored alternate-screen
switches and sent quit before the new main UI was ready. The emulator now saves,
clears and restores buffers for DEC 1049 transitions, with a direct stale-text
regression. No shutdown timeout or product guard was relaxed. Corrected release
PTY plus screen regression: two passed, 3.743 s (session 16850 completed).

Installed archive acceptance passes both terminal and screen checks in 3.652 s
(session 61286 completed): `/tmp/alfredo-terminal-candidate-missions/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256 `f67736b72b5becc3dd64549ab57d78349a97ad8675703cecdf23ed10e9940c43`. All verification processes have completed. Documentation A (98.2%),
diff check clean; no GitHub publication. The unreproduced recovery failure remains
an open stability finding, alongside the remaining product/launch requirements.

## Controlled worker ownership lifetime regression

A deterministic fork test reproduced a worker-owner lifetime defect: with an unrelated
child retaining the inherited file descriptor, dropping the parent's owner still
made recovery report “Worker is still active; recovery refused”. The child uses only
async-signal-safe libc calls after fork and remains alive until after the recovery
observation, so the test does not depend on winning an exec timing window.

Claims and temporary recovery probes now return a non-cloneable `WorkerOwner` guard
around the existing explicit-unlock journal-lock primitive. Orderly scope exit unlocks
before closing the descriptor. Active-owner refusal and retained-evidence validation
are unchanged; missing evidence still returns Outcome unknown without replay. Crash
release still depends on the OS closing all inherited descriptors.

Red: `/tmp/alfredo-worker-owner-red.log`, controlled test failed with the active-owner
message after release. Green: `/tmp/alfredo-worker-owner-green.log`, six recovery tests
passed/one subprocess fixture ignored, including controlled inheritance, live owner,
process death, legacy owner and malformed/missing evidence cases. This proves and
fixes a concrete ownership defect consistent with the earlier intermittent assertion;
its original error text was unavailable, so exact causal attribution is not claimed.

Final full native suite: 123 passed/three ignored; strict Clippy and release build
pass. Release PTY/screen acceptance passes two tests in 4.043 s (session 62196
completed). Documentation A (98.2%). No test expectation or recovery evidence guard
was weakened. Task/conversation schemas and shared host execution are unchanged.

Installed archive acceptance passes two tests in 3.771 s (session 35037 completed):
`/tmp/alfredo-terminal-candidate-worker-owner/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`. SHA-256 `5134e8b76c4ec0c6fed2490ad06bbf2c428ce8a388f59f6452ce33202af6e597`.
All verification sessions completed; no processes pending. Documentation A (98.2%),
diff check clean. No GitHub publication occurred; full production acceptance remains open.

## Saved repositories and cross-directory resume

Open-mode startup now lists saved repository paths through the same bounded identity
scanner used for mission names. Paths deduplicate across missions. Tab fills a path;
Enter performs current exact-root validation and then refreshes mission suggestions.
Missing/moved paths remain hints and are never recreated or accepted by discovery.
Create mode does not fill existing paths. No schema or new registry is introduced.

Discovery tests cover deduplication, workspace isolation, stale hints and non-mutating
reads in addition to existing corruption/symlink/entry/byte bounds. Full Rust suite:
124 passed/three ignored; strict Clippy passed. The stronger PTY launches from the
state directory, then selects saved repository and mission before continuing the
coding workflow. Its first run exposed ready guidance shown during loading: Tab had
no results yet and Enter safely rejected the starting directory. Guidance now
separates loading, empty results and selectable results for both startup steps.
No timeout or repository-validation check was relaxed.

Final guidance correction passes strict Clippy and release PTY/screen acceptance
(two tests, 3.869 s; session 10856 completed). Installed archive acceptance also passes
(two tests, 3.788 s; session 6113 completed): `/tmp/alfredo-terminal-candidate-workspaces/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256 `91dd45f3044d603a0ad2696c73698e54fd24a7ca2cb35ecd72c0595e32f866d4`. All verification processes completed; documentation A (98.2%),
diff check clean. No GitHub publication occurred. Full mission formation and remaining
production-launch requirements stay open.

## Explicit mission identity admission

Contract review separates mission identity from agreed scope. Startup now offers
Resume and Start New as explicit F2 modes. Resume requires a valid mission identity
or legacy task/conversation state; Start New refuses any existing mission/task/
conversation data. CLI --mission resumes; --new-mission creates, and both flags
together are rejected. New mission.json version 1 is published atomically under the
namespace lock, with one winner for concurrent creation. Uncertain acknowledgment
instructs explicit Resume of the same name. Identity grants no task/scope approval.

Legacy task/conversation state resumes without rewriting its source. Advisory
identity.json alone cannot make a mission resumable. Mission files validate exact
workspace/name/version and an 8-KiB encoded bound; JSON-escaped size is checked before
publication. Discovery prefers mission.json. Corrupt/conflicting records are preserved.
Older binaries ignore the new admission record, so mixed-version concurrent creation
cannot provide this new exclusion guarantee.

Verification: full Rust suite 128 passed/three ignored; strict Clippy and release
PTY/screen tests pass (two tests, 4.060 s; session 19500 completed). Tests cover
concurrent starts, missing Resume, duplicate Start New, corruption, advisory-only
records, legacy task/conversation resume, escaped record bounds and conflicting CLI
flags. Real PTY verifies missing/collision refusal, successful saved Resume and
--new-mission creation while continuing the complete coding workflow.

Mission Draft scope agreement, Shared Understanding and the project-level formation
receipt chain remain open; naming a mission does not substitute for those gates.

Installed archive acceptance passes two tests in 3.974 s (session 74088 completed):
`/tmp/alfredo-terminal-candidate-mission-choice/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`. SHA-256 `a22559f867fbf9626718583115a4e5e4762baa2bcfb1873eae93fdf5b979dc43`.
All verification processes completed; documentation A (98.2%), diff check clean.
No GitHub publication occurred. Full mission formation and launch acceptance remain open.

## Explicit project-level understanding gate

The native `/scope` flow retains destination, scope, constraints and known uncertainty
in a workspace-scoped version-1 journal, shared across missions in the same runtime.
Revisioned Mission Commander receipts prove draft/confirmation state on reload. A
replacement draft resets confirmation; confirmation names the exact draft revision
and invokes no next action. `/scope-retry` preserves exact failed-write identity.

Task transactions acquire the scope lock before the task lock and check the gate
before new proposals, saved plans, policy/assignment/approval changes, repairs and
run claims. Worker owner admission checks it before preparation. Exact acknowledged
requests replay without effects; cancellation, existing-run Finish/Review and accepted
branch handoff remain available. Model discussion and transient plan previews remain
noncanonical. Scope confirmation grants no task policy/approval. Journal admission
reserves the count and worst-case encoded bytes needed to confirm an accepted draft.

Verification: full Rust suite 132 passed/three ignored; strict Clippy and release
PTY/screen acceptance pass (two tests, 4.431 s; session 28662 completed). Tests prove
cross-mission refusal, exact replay, stale confirmation rejection, receipt/projection
validation, worker-owner blocking, cancellation, no approval from confirmation, and
last-slot confirmation capacity. PTY records a draft, observes proposal refusal,
confirms the exact revision with no task creation, then continues the coding journey.

Installed archive acceptance also passes (two tests, 4.285 s):
`/tmp/alfredo-terminal-candidate-understanding/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`,
SHA-256 `8d71d7080b1e17544c36be0788bde5f84cb43a3e852d3795545fbb63c752bfed`.
Package and installed-test logs report completion. Documentation validation remains
A (98.2%). No verification processes remain pending.

This is explicit native entry, not complete Wayfinder parity. Automatic Chart/
Work-through first-contact routing, planner consumption/provenance of the agreed
brief, complete mission-formation receipts, gate status in task-readiness summaries
and old-client migration remain open. Task/conversation schemas stay v8/v2; the
legacy desktop's Wayfinder state is not imported or governed by this journal.

## Scope-aware task supervision

Task views now observe project scope alongside the task snapshot and show outside-flow,
pending, confirmed or unavailable state. Proposed/approved task search includes scope
blockers. Visible task views refresh once per second; scope observations never replace
task admission checks. A pending or unreadable scope turns dispatch off, and later
confirmation does not re-enable it. Blocked manual starts do not consume an approval
attempt. Readable task history remains available when the scope journal is corrupt.

The adapter regression covers a draft and confirmation saved in another mission,
rendered confirmation guidance, scope filtering, no automatic dispatch resumption,
unchanged task receipts and corruption. The first full suite caught dependency text
wrapping after scope text was prepended; scope now occupies its own line and the
existing dependency text assertion passes unchanged. Two worker fixtures now refresh
through the normal adapter before starting instead of injecting only a task snapshot.

Final verification: 133 Rust tests passed, three ignored; strict Clippy and locked
release build passed. Release PTY/screen acceptance passed (two tests, 4.880 s;
session 17180 completed). The initial PTY invocation used a relative binary path and
could not launch after changing cwd; the absolute-path invocation passed. Documentation
validation remains A (98.2%). No process remains pending. The prior installed archive
predates this follow-up; it was not rebuilt or published in this checkpoint.

## Scope-bound planner provenance

The native planner captures scope off the UI thread and sends the brief and revision
alongside committed repository context. Provenance comes from the adapter rather than
model output, and appears in the preview and saved Plan receipt; Activity names its
scope revision. Task schema v9 retains the compact binding. Publication and planned
worker owner/Start admission compare it with current scope under the scope lock.
Reconfirmation changes the binding even for identical text. Exact acknowledged replay
and existing-run completion/review preserve their previous boundaries.

Versions v1–v8 remain readable with absent scope binding and receive exact-byte backup
before v9 mutation. Legacy plans cannot gain implicit scope agreement; once explicit
scope begins, their new runs need a fresh plan. The regression covers cross-revision
publication/run refusal, unchanged receipts, exact replay, fresh replacement, forged
schema downgrade, overflow-safe binding validation and actual HTTP scope input.
Migration coverage now includes v8. Strict Clippy initially reported a larger command
variant; boxing the optional binding resolves it without changing serialized bytes.

Verification: full Rust suite 134 passed/three ignored; strict Clippy, formatting,
locked release and PTY/screen acceptance pass (two tests, 4.394 s; session 65892
completed). Installed archive acceptance passes (two tests, 4.354 s; session 20382
completed): `/tmp/alfredo-terminal-candidate-plan-scope/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`,
SHA-256 `67a3b790851330a9d7d404cc33a99344d8a7bf606c984fd253eb787de66b8679`.
Documentation A (98.2%), diff check clean; no verification remains pending.

Automatic Chart/Work-through routing, full manual/repair formation provenance, migration
of desktop scope state and final launch acceptance remain open.

## Coding worker scope handoff

A real HTTP regression proved that the planner's saved scope binding did not reach
the coding request. The worker now includes the exact binding from the acknowledged
Plan receipt alongside approved source context, with explicit reference-only wording.
It does not replace the binding from mutable project scope during an active run or
expand file/check permissions. Manual/repair tasks without Plan lineage retain their
existing context flow rather than receiving fabricated scope provenance.

Red: `/tmp/alfredo-worker-scope-red.log`. Green proves the exact serialized binding
reaches the provider, approved checks pass in isolation, policy remains unchanged and
the source workspace retains its original contents. Full Rust suite 135 passed/three
ignored; strict Clippy, formatting, locked release and PTY/screen acceptance pass
(two tests, 4.370 s; session 71454 completed). Documentation A (98.2%), diff check clean;
no process remains pending. No schema change or new archive; the previous installed
archive predates this handoff fix. No commit, push or publication occurred.

## Browser Playwright acceptance expansion

The current goal explicitly includes browser Playwright tests. The matching Chromium
revision was missing from the default cache; installing revision 1228 in an isolated
`/tmp/alfredo-playwright-browsers` cache enabled actual browser execution without a
package dependency change. Production build and all four fixture-backed layout tests
passed (6.2 s). The real localhost test was extended with page-reload continuity,
verifying canonical restored landmarks, workspace path and Mission tree identity; it
passed (38.4 s). Localhost failures now retain traces and screenshots.

The prototype suite initially passed three tests and timed out on one label check.
An unchanged focused trace passed and measured 20.182 s in navigation, including slow
development dependency requests. The final full prototype rerun passed four tests
(56.4 s), with no timeout or geometry assertion relaxed. The first reload assertion
used the entry screen's landmark; captured state showed successful restoration under
Prompt Workstation, so the test now asserts that actual canonical surface explicitly.

TypeScript check and diff check passed; documentation A (98.2%). All verification
processes completed. These are browser coverage results, not proof of native terminal
parity or complete issue/function acceptance. The new
[browser regression matrix](../Tasks/browser-regression-matrix.md) records exact suite
boundaries, logs, the initial timing failure and remaining functional journeys. No
native source change, commit, push or publication occurred in this checkpoint.

## Native client response timing

Conversation timing now separates submission-to-admission, admission-to-first-nonempty
text, streaming and total elapsed time. A 250-ms active redraw advances the clock even
when inference is silent. Completion, failure and cancellation freeze it; retry starts
a new attempt and saved conversations exclude monotonic clocks. These are UI-observed
intervals, not server processing timings or a claim of faster model execution.

The stronger PTY initially showed admission pending after the HTTP request had begun:
provider admission events were emitted only for queued requests. Both admission paths
now emit Admitted before HTTP. Provider ordering assertions cover immediate and queued
requests; the PTY requires the waiting-for-text clock to advance during silent inference.
Final full suite: 137 passed/three ignored. Strict Clippy, formatting, locked release
and PTY/screen acceptance pass (two tests, 5.406 s; session 43694 completed). Documentation
A (98.2%), diff check clean. No process remains pending or archive was rebuilt.

One intermediate full suite returned outcome-unknown instead of cancelled in the shared
process-cleanup test. The failure is retained in
`/tmp/alfredo-client-timing-cleanup-failure.log`; its assertion now includes the receipt.
The focused rerun and 30 subsequent attempts passed, as did the final full suite.
No cleanup behavior was changed and no cause is established; this remains an open
reliability finding. See `/tmp/alfredo-cleanup-repetition.log`. Browser checks were not
repeated for this native-only UI/provider change; their preceding evidence is separate.

## Identity-preserving cancellation cleanup

The earlier cleanup failure reproduced on concurrent library attempt eight. Its full
receipt reported `process identity changed before forced cleanup`. A controlled test
then proved the cause: SIGTERM stopped the leader, the grace loop reaped it, and a
surviving group member ignored SIGTERM. The later check still assumed the leader had
not been reaped and could no longer verify its identity.

Unix cleanup now retains the unreaped leader through the configured grace interval,
so the existing identity/group checks can run before forced signaling. It then reaps
and proves group quiescence. No new post-reaping signaling path or relaxed identity
check was introduced. The test proves provider cleanup stops the surviving member
without its fallback helper, and a separate test proves mismatched live identity is
refused without signaling that child. This may use the full configured grace interval;
no timeout bounds or execution permissions were expanded.

Evidence: `/tmp/alfredo-cleanup-parallel-reproduction.log`,
`/tmp/alfredo-cleanup-deterministic-red.log`,
`/tmp/alfredo-cleanup-identity-preserved-green.log`, and
`/tmp/alfredo-cleanup-parallel-verification.log` (100 full concurrent library runs).
Final native suite 139 passed/three ignored; strict Clippy, formatting, locked release
and real PTY/screen acceptance passed (two tests, 6.035 s). Shared Rust compatibility:
68 passed/one ignored. Python execution compatibility: 83 run/one skipped. The original
unexplained failure is now backed by reproduction, cause and verification.

Installed archive acceptance passes (two tests, 5.294 s):
`/tmp/alfredo-terminal-candidate-cleanup/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`,
SHA-256 `e67a16500b255c76bfcba207580b0e1dcbc2ee4663d147cf831dacb10203d1fc`.
Documentation A (98.2%), diff check clean. All verification processes completed;
no commit, push or publication occurred. Browser evidence remains separate because
this change affects shared process execution rather than browser rendering.

## Live-model observations and thinking progress

Real local checks passed through the Rust provider and isolated coding worker on
Ollama 0.30.6. The initial qwen2.5-coder:14b chat completed in 18.914 s (server load
18.74 s), followed by a passing coding check in 2.024 s. The initial default qwen3:14b
chat completed in 22.711 s (server load 20.22 s); its coding check passed in 8.481 s.
Three later resident-model requests returned READY in 2.102–2.953 s, with thinking
progress first observed after 0.282–0.462 s. The model digest remained unchanged.
These synthetic fixtures provide observations, not sustained qualification, model
ranking or a claim that this patch sped up inference. Loading dominated these initial
request delays. Model/version/residency data and logs are retained in the
[live-model observation artifact](2026-09-13-live-model-observations.json).

Following the [Ollama thinking-stream contract](https://docs.ollama.com/capabilities/thinking),
the provider now accepts thinking-only frames and emits one payload-free progress
event per request. Conversation, planner and worker projections distinguish thinking
from initial loading and final answer content. Reasoning never becomes transcript or
saved model-response text. Both thinking and answer bytes count toward the existing
128-KiB output bound; frame size stays bounded at 64 KiB. Progress is transient and
attempt-bound, preserving retry/cancel/restart behavior and current schemas.

Final deterministic suite: 142 passed/three ignored; strict Clippy and locked release
passed. The strengthened real PTY/screen journey passed (two tests, 5.469 s), explicitly
showing thinking progress without its private fixture text while a second session
completes. Updated live Rust tests also observed the progress signal from qwen3.
Documentation A (98.2%), diff check clean. All verification completed. No default model
or generation settings changed; no archive rebuild, commit, push or publication.

## Stale-plan readiness

Stale-plan readiness checkpoint: observed scope revision changes now produce a separate
selected-task blocker and searchable explanation for unstarted planned tasks, including
legacy plans without a scope binding once explicit scope begins. Manual start refuses
before recording an approval attempt; dispatch skips those tasks and can choose a later
eligible plan. The journal still revalidates the full binding under its lock; UI
observation does not grant authority or rewrite history.

Focused planner suite: 11 passed. Full native suite: 143 passed, three optional live tests ignored. Strict Clippy and release build passed; release PTY: two passed in 5.437 s. Documentation A98.2; formatting and diff checks clean. Evidence: `/tmp/alfredo-stale-plan-{focused,full,clippy,release,pty,docs}.log`. No package rebuild or publication.

## In-app workspace and mission handoff

The `/workspace` command reuses validated repository selection and explicit
Resume/Start New admission without restarting the terminal. Cancelling returns to
the current state. A Workstation owns the App, TaskControl and conversation owner;
target state opens first, then an ordered final source save precedes replacement.
Failures preserve the current drafts and ownership; target ownership releases on a
failed source save. Fresh event channels isolate restored session/attempt IDs.
Dispatch defaults off in the selected mission. Active chats/workers, dispatch,
pending task operations/catalog discovery and unsaved plans require resolution.

Three focused tests pass for per-mission model/draft/task-view restoration, old-save
ordering, unchanged task journals, owner contention, failed final save and quiescence.
The first full suite exposed worker-progress clipping at 60×25 after adding the
identity header. A diagnostic render showed the task panel also repeated workspace
identity. Removing that duplicate restores progress visibility; the tightened test
now checks both mission identity and waiting status. Red/green evidence is retained
in `/tmp/alfredo-workspace-layout-{red,green}.log`.

Final native suite: 146 passed, three optional live tests ignored. Strict Clippy,
release build and expanded release PTY pass (two tests, 5.939 s). The installed archive
passes the same acceptance journey (two tests, 5.922 s). Candidate:
`/tmp/alfredo-handoff-tyw6PL/candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256: `4b289c00f51f9ee65ba2b11fb1d9ac15180297fa71fe3790cd95cf2edbb330da`.
Logs: `/tmp/alfredo-workspace-{focused,full,clippy,release,pty,package,installed,docs}.log`.
The packager refused a pre-existing output directory before building; a fresh temporary
candidate directory was then used. All verification processes completed. Formatting
and diff checks pass. Browser/shared-provider sources were not changed this checkpoint.

Full cross-workspace background supervision and Workspace Session receipt parity
remain open; this is explicit switching between quiescent work contexts.

## Native Wayfinder first contact

Native conversation turns now pass through one deterministic Wayfinder adapter before
model dispatch. New projects/consequential changes enter Chart; explicit Wayfinder
references enter Work-through; read-only questions retain the legacy entry exclusion.
The durable flow and pending gate are shared across missions. Four labeled scope
fields save a revision-bound draft; explicit `confirm shared understanding N` records
Commander agreement and ends the turn without inference, tasks or delegation.
New-project `/plan`, `/task` and `/after` shortcuts also enter this adapter first.

Understanding v2 adds validated flow/Enter receipts and preserves exact v1 bytes before
mutation. Startup/replay grants no new confirmation. Entry placeholders require a real
scope draft before confirmation. Model prose cannot call the scope transaction path.
Routing jobs remain tracked after conversation cancellation; switching and normal quit
wait for the saved outcome. Dispatch pauses while routing is pending and stays off when
the scope gate closes. No task v9 or conversation v2 schema change.

Focused tests cover entry vocabulary, cross-mission continuation, simultaneous entry,
placeholder/stale confirmation refusal, exact v1 migration/backup conflicts and scope
writes after conversation cancellation. Two further failing regressions exposed a
historical-confirmation reply pointing to the newest receipt and client timing scrolling
out of long history. Reply construction now identifies the original receipt plus current
gate; timing is pinned outside transcript scrolling. Red logs:
`/tmp/alfredo-wayfinder-{replay,timing}-red.log`; focused corrections pass.

Final native suite: 152 passed, three optional live tests ignored. Strict Clippy and
release pass. Expanded release PTY: two tests passed in 6.153 s; installed candidate:
two passed in 6.115 s, including exact captured-scope model-input assertions. The
journey verifies zero model requests and zero task creation for entry/draft/confirmation,
then retains the coding/review/restart/workspace-switch paths. Candidate:
`/tmp/alfredo-wayfinder-lHVglq/candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256: `9512b9958e894d9bfc8bdb1ee64e2d0872939f650e1f786f477c87e9d3d5d383`.
Logs: `/tmp/alfredo-wayfinder-{full-final,clippy-final,release-final,pty-final,package,installed,docs}.log`.
Documentation A98.2; formatting/diff checks clean. All verification processes completed.
Shared execution and browser implementations were unchanged this checkpoint.

Structured per-message capability attribution, complete Mission Draft/graph/skill
execution and desktop/native migration remain unfinished. Existing actor-specific
scope receipts are not full conversation-source metadata.

## Structured response attribution

Conversation v3 retains a per-assistant-message source map: requested model name or
Wayfinder adapter with an optional original scope receipt reference. The UI renders
these labels independently of message text. Model output that imitates Wayfinder stays
model output; retry clears only the interrupted response's source and attempt guards
reject late relabeling. Legacy replies retain `source unrecorded`, and source metadata
never enters provider role/content messages or grants scope/task authority.

Both v1 and v2 migrations preserve exact version-named backups and refuse conflicts,
downgrades, invalid indices/kinds/bounds or metadata claimed by an older schema.
Focused source/retry/wire-format and storage/migration tests pass. Final native suite:
155 passed, three optional live tests ignored. Strict Clippy/release pass. An initial
PTY assertion used a non-ASCII Python byte literal; correcting the harness syntax
allowed the same release binary to pass both terminal tests in 6.139 s. This journey
checks a visible receipt label and durable Wayfinder/model records alongside coding,
restart and workspace switching. Installed acceptance passed: two tests in 6.136 s.
Candidate: `/tmp/alfredo-attribution-q2FTDQ/candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256: `8366d6e4430aa6818a331f9f192b710fbff3c2e0fd38d151a5d47264a87bf20f`.
Evidence: `/tmp/alfredo-attribution-{focused,full,clippy,release,pty,package,installed,docs}.log`.
Documentation A98.2; formatting/diff checks pass. All verification handles completed.
Shared execution/browser implementations were unchanged this checkpoint.

Full cross-capability workspace/mission/task action chronology and qualified model
profiles/digests remain unfinished. Sources are historical presentation provenance;
current authoritative journals still determine all actions.

## Stable reading during streamed output

The red render regression proves fixed bottom offsets displace old text: appending
40 new lines replaces the previously visible HISTORY_032 with no history. A transient
per-session logical-line/wrapped-row anchor now preserves the reading point while
output grows. PageDown advances from that point and follows again at the bottom;
new/retried turns follow immediately. Hidden task/model views no longer render or
modify the conversation viewport. Resize retains the logical line with a clamped
wrapped offset.

Transcript rendering now skips preceding logical lines before using the widget's
16-bit scroll. A bounded 66,000-newline regression verifies both actual tail visibility
and navigation back to the start. Focused tests also cover growth, resize, session
switching, follow-latest and transient-cache exclusion from snapshots. Native suite:
157 passed, three optional live tests ignored; strict Clippy/release pass. Controlled
HTTP/PTY journey: two tests passed in 6.298 s. It scrolls up, waits until 40 additional
streamed lines reach saved UI state, toggles through the task view, verifies the same
history anchor and returns to live output with PageDown. Existing coding/review/restart
and workspace-switch journeys also pass. Installed archive acceptance passed: two
tests in 7.090 s. Candidate:
`/tmp/alfredo-reading-lttrNc/candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz`.
SHA-256: `a124b3ab04bc36ceeafa5ee4a9944abd0d886bf8089f45dc4bcfdfd3af8b341a`.
Logs: `/tmp/alfredo-reading-{focused-final,full,clippy,release,pty,package,installed,docs}.log`.
Documentation A98.2, formatting/diff checks clean; all verification handles completed.
Shared execution/browser sources were unchanged this checkpoint.
Red evidence: `/tmp/alfredo-reading-red.log`.

Conversation v3 still stores its compatible numeric bottom offset. Exact persisted
anchors across changed restart geometry or unseen background growth, and character-
level reflow positions, remain open rather than being inferred from live-view tests.

## Remaining acceptance

This is a working isolated coding slice, not the finished product. Native issue-graph
scheduling, automatic repair routing, complete workspace/mission/view continuity,
model roles/global capacity, retirement and aggregate disk budgets, final
issue-to-regression acceptance, keyboard/human accessibility review, sustained
multi-agent coding acceptance, comparative speed measurements, non-Linux support
and remote release acceptance remain required. Accepted dependencies now compose
into verified isolated baselines; conflict repair remains open. Checks can retain
untracked build output; failed worktrees and managed refs are not automatically
removed. Crashed claims lacking valid saved evidence remain uncertain without
automatic replay. Complete saved terminal results can be explicitly recovered.

The original full regression observations above remain historical evidence for
untouched legacy paths, including the unresolved WSL retirement inspection failure.
No commit, push, publication, visibility change or completion claim was made.

The persistence SOP was corrected from an unrelated Flutter/sqflite template to
the repository's actual versioned-JSON migration procedure. No Dart application
or sqflite database exists in the inspected source tree.

## Installed-model argument completion

Model-argument completion checkpoint (2026-09-14): Tab after /model PREFIX or
/assign ID PREFIX opens filtered, sorted, deduplicated installed names; Enter
only fills the draft. Missing names/catalog produce a notice without switching
sessions or fetching. Mid-draft completion refuses replacement. Existing submit
validation still protects active/interrupted model selection and resets approval
on reassignment. Native 160 passed / three optional ignored; strict Clippy and
release pass. Installed PTY acceptance passes, including no model selection or
assignment receipt until the completed draft is submitted. Unicode, invalid IDs,
empty catalogs, no-match and cycling are covered. Full capability routing and
model qualification remain open. Candidate /tmp/alfredo-completion-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 c7b76388e08eb46aa6e257fb9e01db00c1bd8eb54a1171e09ca5cc123b66a932.

Installed acceptance output:

```text
..
----------------------------------------------------------------------
Ran 2 tests in 7.466s

OK
Installed archive acceptance passed: alfredo-tui 0.1.0 (x86_64-unknown-linux-gnu)
```

Generation provenance and repeated-worker checkpoint (2026-09-14): worker evidence
now records requested thinking mode, token limit and temperature at request preparation.
Verified review displays the settings; missing legacy records remain unrecorded.
Malformed settings are rejected, and changing settings in saved evidence fails the
existing digest check. This does not prove dispatch or server compliance and adds
no task authority or snapshot version change.

Native 163 passed / four optional ignored; the subsequent explicit-on/tamper worker
regression, strict Clippy, release packaging and installed PTY passed (two tests,
7.530 s). All 35 packaged source hashes match the current checkout. Candidate:
/tmp/alfredo-generation-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz;
SHA-256 acc65ccc44196d23cadf5062adbe199a3dac4f02c8df2990e389313a1210c2cb.

Five sequential pairs of live qwen3:14b coding tasks passed all independent checks
and produced review-ready candidates without changing source workspaces. Server
version and model digest match before/after. First-pair completions took 12.100 and
16.370 s, with approximately 9.25 s model loading. Warm port tasks took 2.690–2.735 s;
warm interval tasks took 6.544–6.587 s, including upstream queueing. Ten small tasks
are repeated-workload evidence, not broad role qualification or a long-duration soak.
See [exact identities, samples, source hashes and verification logs](2026-09-14-generation-and-repeated-workers.json).
Full formation/chronology/retirement, broader model workloads, cross-platform and
launch acceptance remain open.

Plan revision checkpoint (2026-09-15): `/plan-revise REQUEST` sends the previous
unsaved task list, accumulated user requests and freshly captured committed context
and scope to the original planner model. A validated replacement still requires
explicit `/plan-save`, then separate task approval. Failed/incomplete revisions
restore the old draft and original revision; stale saves remain rejected. Cancel
removes both drafts and ignores late output. Invalid requests preserve the draft
without dispatch. Reference tasks are bounded to 64 KiB and prompt history to 8 KiB.

Four added regressions cover actual HTTP context, selected-model stability, explicit
save/approval, malformed/incomplete output, stale restoration, cancellation and
invalid requests. The installed PTY now revises before saving, verifies no earlier
task mutation and checks the saved replacement. Restored the ordinary review notice
that the existing terminal regression expected; revised completion is distinct.
Native 167 passed / 4 optional ignored, strict Clippy and installed
archive acceptance pass. Candidate /tmp/alfredo-plan-revise-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 65baef3807452cbb9a5a67023b8f9d12055d9966ed14b969093a54746d35e5d8.
All packaged source hashes match the current checkout. Exact logs are in the
[verification report](2026-09-15-plan-revision.json).

The explicit user continuation resolved the automatic approval rejection. Draft
restart persistence and full Mission Draft/Issue Graph formation remain open;
this refinement workflow does not imply full issue or launch completion.

Plan continuity checkpoint (2026-09-15): conversation v5 persists complete review
plans with their original task revision. Checkpoints during refinement retain the
previous complete plan; no partial first draft or inference is resumed. Quiescent
workspace/mission handoff preserves valid drafts and restores their review view.
Malformed or oversized plans and old-version plan fields are rejected. V1–v4 remain
readable and exact-byte backups precede the first v5 write; conflicts/downgrades
refuse unchanged. Task v9 and scope v2 are unchanged.

Migration tests prove v4 backup conflict handling, exact preservation and malformed
payload refusal. Workstation tests prove restart, workspace isolation, switch-back,
stale-save refusal and cancellation removal. The installed PTY exits with a revised
plan, restores it without another model request, and explicitly saves the replacement.
Native 169 passed / 4 optional ignored, strict Clippy and installed
archive acceptance pass. Candidate /tmp/alfredo-plan-persistence-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 434f45a978b9336bb88dea428b71b34082225bd98491c9f8a04d9a44a1ee4660.
All packaged source hashes match the current checkout. See the
[verification report](2026-09-15-plan-continuity.json).
Full mission formation, chronology, recovery/retirement and launch remain open.

Capability discovery checkpoint (2026-09-15): `@wayfinder REQUEST` addresses the
native scope adapter. Tab/arrow/Enter completion fills the draft without submitting;
Escape closes the picker and F1 lists the capability alongside commands. Unknown
leading capability names and empty requests fail without model dispatch or draft
loss. Explicit discussion can enter Chart/Work-through, and prefixed four-field
briefs/confirmation use the existing exact receipts. No task approval is inferred.

New tests exposed a real 32×10 layout failure: the session list consumed the entire
content area. Completion now gets that area while open and hidden conversation
rendering cannot update its reading anchor. The original narrow assertion passes.
Installed PTY verifies unknown-name refusal, completion/dismissal/no early action,
explicit scope entry, brief and confirmation without model inference or task effects.
Native 171 passed / 4 ignored; strict Clippy and installed archive
acceptance pass. Candidate /tmp/alfredo-wayfinder-capability-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 397d8f92ae790c9c7e31b82af4add123679e4e978b0742aee9864d6baa261d03.
All packaged sources match the checkout; [exact evidence](2026-09-15-wayfinder-capability.json).
Only the existing Wayfinder adapter is exposed; general skill routing, full mission
formation/action chronology and remaining production launch requirements stay open.

New-workspace baseline checkpoint (2026-09-15): explicit Create repository now
initializes an empty root commit after sanitized Git init. The selector discloses
this before creation. Git mktree uses null stdin, commit-tree uses the Alfredo
identity with signing disabled, and update-ref publishes main only if absent.
No project files or index are staged; existing repositories are never auto-committed.
Partial creation remains inspectable and never reports successful acknowledgement.

Selection tests verify a root commit with empty context, no source files, no mission
state and preservation of existing unborn repositories and their untracked files.
A real HTTP worker test creates the first files in an isolated worktree, passes
its approved check and retains a review-ready candidate while source HEAD/tree
remain empty and unchanged. The first fixture expected old source text; corrected
to require explicit new-file context without weakening existing-source tests.
Installed PTY verifies disclosure before creation and the actual empty Git tree.
Native 173 passed / 4 ignored; strict Clippy and installed archive
acceptance pass. Candidate /tmp/alfredo-new-workspace-candidate/alfredo-tui-0.1.0-x86_64-unknown-linux-gnu.tar.gz; SHA-256 6d2957e9dfda76066f2d4689a3e7cbfaf39cc029ec7670284e92ea23d8c87fd0.
All packaged sources match; [verification report](2026-09-15-new-workspace-baseline.json).
Uncommitted context import, full workspace receipts/mission formation and remaining
production launch acceptance stay open.


Recovery update (2026-09-20): synced task/run/baseline markers allow explicit
recovery of a stopped worker as Failed only when no check-launch intent or final
evidence exists. Real subprocess crash and refusal/idempotency/repair tests pass;
native suite 176 passed / 5 ignored, strict Clippy and installed PTY pass. Partial
work remains retained. Possible check launch remains unknown without ordinary
terminal evidence; this does not prove helper quiescence or authorize retirement.
[Evidence](../Reports/2026-09-20-precheck-recovery.json).


Terminal panels update (2026-09-20): Models and Activity content remain reachable
at 32×10; shared task-panel logical-line slicing reaches evidence past 65,535
rows. Page navigation clamps after end/resize. Native 178 passed / 5 ignored,
strict Clippy and installed PTY pass. This advances terminal usability coverage;
full chronology and release acceptance remain open.
[Evidence](../Reports/2026-09-20-terminal-panels.json).


Evidence-render update (2026-09-20): cached wrapping of immutable evidence and
visible-page cloning reduce the optimized synthetic 100-redraw workload from
539.962 ms to 17.425 ms. Unicode/style/resize buffer equivalence and work-count
regressions pass; native 180 passed / 6 ignored, strict Clippy and installed
PTY pass. This does not qualify model or end-to-end latency.
[Evidence](../Reports/2026-09-20-evidence-rendering.json).


Acceptance-contract update (2026-09-20): #4 native planned task packets now retain
bounded explicit acceptance criteria through review, save, restart, worker requests
and separately approved repair descendants. Task v10/conversation v6 migrations
retain exact originals; absent legacy criteria stay unrecorded. Fixed criteria
displacing live progress and discovery omitting the new schema. Native 183 passed
/ 6 ignored, strict Clippy and installed PTY pass. Full mission formation,
criterion-level review and chronology remain open.
[Evidence](../Reports/2026-09-20-plan-acceptance.json).


Criterion review update (2026-09-20): #7/#39 native acceptance/rejection now supports
explicit reasons and ordered per-criterion evidence notes via /review. Contracted
acceptance requires all met; historical boolean decisions gain no inferred notes.
Task v11 migration, exact replay, evidence tamper refusal, visible acknowledgment
and separately approved repair context pass. Native 185 passed / 6 ignored,
strict Clippy and installed PTY pass. Limited approval, human escalation and tiered
automatic routing remain unfinished.
[Evidence](../Reports/2026-09-20-criterion-review.json).


Dependency gate refresh (2026-09-20): current RustSec database (1,251 advisories)
reports no known vulnerabilities or warnings across 213 locked dependencies, with
no ignored advisories. The vulnerable fixture failure gate and nine notice tests
pass. Current criterion-review candidate source/payload hashes and its exact audited
lockfile match; no product changes or rebuild were needed. Application security,
license compatibility, remote CI and full release acceptance remain separate.
[Evidence](../Reports/2026-09-20-native-dependency-audit.json).

## 2026-09-20 explicit review outcomes

Native review now records all five outcomes, including limited approval and a
human-review hold that requires explicit resolution. Both approvals require the
original successful check and all recorded criteria met. Schema v12 preserves
exact older-store backups and historical reviews. Regression coverage exercises
all outcomes with isolated workers, failed-run approval bypass refusal, dependent
dispatch blocking, note replacement and the installed terminal journey.

Validation: 189 native tests passed, six intentionally ignored; final affected
suites, strict Clippy and installed release smoke pass. MIT and third-party notices
are bundled. Exact candidate/hash evidence is in
`.agent/Reports/2026-09-20-review-outcomes.json`. Automated tiered repair/routing,
formation/chronology, recovery/retirement and production qualification remain open.

## 2026-09-20 repair-hold regression

Reproduced and fixed a sibling-repair bypass while an existing child awaited human
review. New proposals refuse under the task lock. Historical v12 receipts remain
readable and exactly replayable; no schema migration. Real isolated-worker tests
cover restart, unchanged storage on refusal, historical replay and explicit hold
resolution followed by a separately approved repair. Native 190 passed / 6 ignored,
strict Clippy and installed PTY pass. Exact candidate and hashes:
[repair-hold report](../Reports/2026-09-20-repair-hold.json).
Tiered routing and full production qualification remain open.

## 2026-09-20 typed risk escalation

GitHub #7's critical/security/merge-risk route now operates on explicit native
reviews. Any declared risk on a nonapproving decision holds the task for human
review and blocks repair/dependent dispatch. Approval carrying risk refuses;
explicit resolution retains original successful-check and criterion requirements.
Task v13 preserves exact old-store backups and unclassified reviews; older schemas
cannot claim classified risk. Activity and evidence show the persisted escalation.

Validation: 192 native tests passed / six ignored, strict Clippy and installed PTY
pass. The real worker matrix covers all three risk classes on successful and failed
runs, restart/retry/conflicts, blocked bypasses and resolved dependent dispatch.
[Evidence](../Reports/2026-09-20-risk-escalation.json). Same/fresh-agent and Architect
routing, automatic model review, full formation/chronology, recovery/retirement and
production launch qualification remain open.

## 2026-09-20 Local Agent conversation continuity

New worker evidence binds a bounded retained user/assistant conversation and
mission-local agent identity. First repair sends the prior exchange; second/later
terminal rejection in ancestry starts a fresh agent with current repair evidence.
Needs repair does not count as rejection. Current policy and fresh approval remain
required. Missing/corrupt referenced history refuses before run claim; legacy or
incomplete history, changed models and budget rollover are explicitly fresh.

Native 197 passed / six ignored, strict Clippy and installed PTY pass. Real HTTP
regressions verify exact history, identity, restart and refusal paths. An independent
subagent reviewed correctness and added edge coverage. Artifact transcript schema1;
task13/conversation6/scope2 unchanged. [Evidence](../Reports/2026-09-20-agent-continuity.json).
Automatic review-triggered repair proposal/launch and Architect routing, model
qualification, full formation/chronology, recovery/retirement and launch remain open.

## 2026-09-20 atomic review-to-repair proposals

No-risk NeedsRepair/Rejected reviews now atomically record the parent outcome and
an inherited Proposed repair child. Fresh approval is still required. Risk holds
create no child. Parent and child Activity searches expose the receipt, and parent
readiness links the child. Task v14 preserves exact backups and old review meaning.

Native 200 passed / six ignored, strict Clippy and installed PTY pass. Subagent
coverage verifies complete atomicity/refusal/replay and actual continued/fresh model
requests, including a failed parent with an already human-held child. The installed
journey verifies the single review command and absence of immediate inference.
[Evidence](../Reports/2026-09-20-atomic-review-repair.json).

The design review identified an outstanding graph gap: accepting a repair does not
resolve dependents referencing the rejected original task. Governed replacement,
Architect routing and the broader product/launch gates remain unfinished.
