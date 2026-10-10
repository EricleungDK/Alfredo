# Alfredo terminal reference (detailed behavior)

First working slice of the [Rust migration](../../.agent/Tasks/rust-terminal-migration.md).
It provides concurrent streaming Ollama conversations and a persistent Rust task
queue in a native terminal. Tasks support dependencies, explicit approval and
cancellation, with receipts that survive restart. Coding execution is
available through explicit file/check policy, isolated Git worktrees and durable review evidence.
Conversation transcripts, drafts, model choices, cursor positions and selected
conversation now restore after restart.

## Autopilot

Type `/go GOAL` (or launch with `--go "GOAL"`) to run the whole loop without further
commands: plan → `/plan-save` → approve every planned task → `/dispatch on`. When a
worker's approved check passes and its evidence verifies, autopilot records an
approved criterion review (reason `autopilot: check passed`). A failed or invalid
run gets a linked `/repair` with the failure summary as reason, bounded by
`--max-repairs N` per task (default 3, 0 disables); an accepted repair is
`/resolve-repair`ed so dependents proceed. Each repair prompt starts with a short
"What is still failing" section (failing test names, assertion/error lines and
`-`/`+` diff lines, at most 30 lines / 2 KiB) before the full prior evidence. A
failed repair whose files equal its parent attempt's is recorded as `No change
from previous attempt`, and the next repair says so and starts a fresh Local Agent
conversation. Repair sampling temperature
stays 0 for the first repair, then steps 0.3 → 0.6 → 0.8 (cap) after two or more
failed attempts, one extra step after no progress. When a reply hits the
4096-token limit (`Model output hit the 4096-token limit`), the next repair
requests 8192. Evidence records the requested temperature and limit. An exhausted task stays failed, its
dependents stay blocked, and independent work continues. Invalid plans are retried
once with the validation error appended, then autopilot stops.

Each choice is an ordinary console command saved and dispatched through the same
intent path as typed input; policy, evidence, locks and receipts stay authoritative.
Risk-classified or human-hold reviews always wait for you, and manual commands keep
working. `/pause` or F5 stops new starts and decisions (running workers finish);
`/resume` or F5 continues; `/stop` also cancels running workers; `/autopilot` shows
status. The header's second row shows the run: state, `done/total`, failures and
repairs when present, elapsed time and branch; the goal is the work group title.
State lives in a small
`autopilot-<conversation-sha256>.json` beside the task store; after restart the loop
is restored **paused** and nothing is replayed until you resume (runs cancelled by
quit or `/stop` are repaired after resume).

When every planned task is accepted, held or failed, autopilot composes the accepted
candidates on the plan's recorded baseline with the dependency merge-tree composition
and creates one local branch `alfredo/go-<id>`. HEAD, the index and working files are
untouched; nothing is pushed. The summary lists each task's outcome and the `git switch`
/ `git merge` commands, and says when the branch holds only an accepted subset.

Scope: a confirmed scope, or no scope with an ordinary goal, is used as is. A new-project
goal (the case that would otherwise enter Wayfinder) records and confirms a minimal
goal-derived scope through the normal scope transactions. A pending scope draft you
wrote is never confirmed for you: `/go` asks you to review it first.

## Build and install a development archive

On Linux x86-64 with Rust 1.96.0 and Python 3.11+, build a native locked candidate:

```bash
python3 alfredo-tui/scripts/package_release.py --output /tmp/alfredo-candidate
python3 alfredo-tui/tests/release_smoke.py /tmp/alfredo-candidate/*.tar.gz
```

The output directory must be new. The archive contains the terminal binary,
[standalone installation instructions](../INSTALL.md), Cargo.lock, BUILD.json,
DEPENDENCIES.json, THIRD_PARTY_NOTICES.txt and the project LICENSE. Build metadata records source
fingerprints, commit/dirty state, compiler and payload checksums. The dependency
inventory includes the target-filtered resolved build/dev graph. Notice bytes come
from checksum-verified cached crate archives, including nested attribution files.
This inventory does not identify linked code or decide license compatibility;
Alfredo’s own code uses the [MIT license](../../LICENSE); dependencies retain their own terms.
A companion SHA-256 file checks archive integrity. Fixed archive metadata makes
repeated packaging of identical inputs deterministic; this is not a cross-host
reproducible-build claim. The builder explicitly selects its qualified native
target, rejects other hosts/toolchains and never publishes. The smoke installs
the extracted binary into a temporary PATH outside the checkout and runs the full
PTY coding/restart/repair journey. Python is a build/test dependency only.

The GitHub workflow builds and retains a candidate artifact after these checks;
remote CI has not been observed in this local run. Current qualification is Linux
x86-64 GNU on this build host; older glibc, other operating systems, dependency
license review and full product acceptance remain open.

Responses identify their source separately from their text: the requested model name,
or Wayfinder with a scope receipt reference when one was acknowledged. Labels survive
restart. Older replies show “source unrecorded”; model wording cannot create a
Wayfinder label. These labels do not grant approval or replace current task evidence.

PageUp lets you read older output while a response continues streaming. PageDown
moves toward the latest text and resumes following when it reaches the bottom.
Each conversation keeps its own reading point while you switch sessions or views.

## Run an explicit inference diagnostic

The standalone runner is opt-in and uses isolated repositories and missions. It
performs real inference and governed worker/check/review actions, requiring the same
Linux worker tools as interactive coding. Choose a new report path; do not pass
workspace, mission or doctor flags:

```bash
alfredo-tui --model qwen3:14b --qualify-inference ./inference-check.json --qualification-repetitions 3
alfredo-tui --inspect-qualification ./inference-check.json
```

Repetitions range from 1 to 3, defaulting to 3. Four scenarios (small edit,
required-source multi-file work, repair, queued foreground work) compare baseline
requests with explicit foreground context 8,192/background context 16,384. Both
profiles retain the other native request settings. The experiment uses one shared
client slot, at most 24 scenario executions and 128 generation requests, and a
1,800-second cohort deadline plus required cleanup. A conflicting live capacity
refuses normally (it does not wait for another capacity to drain). Ordinary production defaults, including capacity two, stay unchanged.

Reports and `<report filename>.artifacts` retain bounded observations, isolated
workspaces and review evidence beside the chosen report. Existing paths refuse;
incomplete checkpoints never resume generation. `--inspect-qualification` validates
and summarizes a saved artifact without HTTP or task effects. A finished cohort may
contain failed fixtures; generated output and ReviewReady are not canonical acceptance.

Actual payload/profile/message/prefix hashes omit raw prompts from request records.
Post-generation runtime inspection runs before recorded completion while the shared
slot is held. Separate generation and metadata-probe clocks expose instrumentation
overhead; reviewed scenario latency includes it. Missing or drifting runtime proof
stays explicit. Hashes cannot reconstruct the original payload or attest an upstream
runtime; the binary pin and exact templated token headroom remain unknown. There is
no profile promotion or general speed claim. Deterministic native and standalone
CLI regressions cover all four scenarios, cancellation, exact wire observations and
rejection of inconsistent reports; live model quality is a separate diagnostic.
See the [qualification plan](../../.agent/Tasks/native-inference-qualification.md) for
authority, legacy context comparisons and the remaining qualification boundary.

## Establish scope in conversation

Describe a new project or consequential architectural change to enter Wayfinder
Chart mode. A reference such as “Wayfinder ticket #42” enters Work-through. Ordinary
read-only questions stay outside automatic entry. A saved flow continues across
missions and restarts. New-project `/plan`, `/task` and `/after` requests also enter
this flow before creating work.

Supply the reviewed scope as four labeled lines (paste them, or use Shift+Enter):

```text
Destination: A working local scheduler
Scope: Local tasks and their dependency checks
Constraints: Keep existing task histories readable
Uncertainty: Sustained model performance still needs measurement
```

The terminal saves a draft and displays its revision. Review `/scope`, then send
`confirm shared understanding N` with that exact revision. Confirmation ends the
turn; use `/plan REQUEST` separately when ready. Entry, draft and confirmation
responses come from saved receipts and require no model inference. Other discussion
uses your selected model with captured scope as reference. Scope agreement never
substitutes for task policy, approval, execution evidence or acceptance.

## Switch work without restarting

Use `/workspace` to choose another repository and open or create a mission.
Esc cancels the selector and returns to your current work. The header identifies
its mission and repository. Each mission restores its own conversations, drafts,
models, task filter and selected task; automatic dispatch starts off.

Finish or cancel active chats/workers, turn `/dispatch off`, and save or cancel a
plan draft before switching. Pending task operations must finish first. The
terminal saves an exact selection request, loads the destination and saves the
current conversations before changing workspaces. An unavailable, occupied or
corrupt destination, or a failed save, keeps your current work open. The originating
conversation retains the selection; the destination records its arrival. Created
repositories and missions survive a later failed handoff and remain identified in
history. Choosing the current mission records **Already current** without reopening it.

## Run

Requires Rust 1.96+ and an interactive terminal. No Node, Python or browser
is involved in this binary. Coding workers currently require Linux, Git, Bubblewrap
and prlimit; an approved check may itself require its named interpreter or tool. Ollama and the selected model are needed to send prompts;
the terminal opens even when the model server is unavailable.

```bash
cargo run --locked --manifest-path alfredo-tui/Cargo.toml -- --model qwen3:14b
```

Use `--endpoint http://127.0.0.1:11434` to select the server. `ALFREDO_MODEL` and
`OLLAMA_HOST` supply defaults; explicit flags win. Prompts go to that endpoint.
The HTTP client ignores proxy environment variables and does not follow redirects.

Alfredo processes owned by the same user share a client inference limit for each
normalized endpoint origin, even with different mission state directories. The
default is two active requests; `--parallel-models N` sets 1 to 8. A request whose
capacity differs from the live one waits, holding no queue ticket and sending
nothing, until the live requests drain; it then adopts its own capacity. The chat
shows **Waiting for another Alfredo process (capacity N) · Esc cancel** beside a
kaomoji cat running along a dotted track (`(=^･ω･^=)`, paws alternating, one cell
per animation frame; static with `--no-motion`, `=^.^=` with `--icons ascii`,
clipped in narrow panes). Esc stops the wait at once. Corrupt or unsafe ledgers,
missing owner proof and a full queue still fail closed. Use the same endpoint
spelling across terminals: normalization does not merge DNS names or aliases such
as `localhost` and `127.0.0.1`.

Discussion and planning use foreground priority; coding-worker model requests use
background priority. Each class is FIFO, and a waiting background request gets a
grant after at most three foreground grants. Active requests are not preempted.
Waiting requests show **Queued for Alfredo**, with observed queue position and
active/configured slots when available. Position may change as foreground requests
arrive. **Waiting for model server** follows admission and does not claim the server
is loading or busy. Model discovery bypasses this queue.

### Connection and warm models

- `--keep-alive VALUE` (env `ALFREDO_KEEP_ALIVE`, default `30m`) is sent as Ollama
  `keep_alive` on chat, planner and worker requests. Accepts Go durations (`30m`,
  `1h30m`), integer seconds (`300`; `-1` keeps the model loaded), or `default` to
  omit the field and use the server setting.
- At startup, on workspace switch and on `/model NAME`, Alfredo preloads the model
  in the background (`POST /api/generate` with an empty prompt). Preload bypasses
  the inference queue; failure is only a status.
- `--connect-retries N` (0–10, default 3; 0 disables): a model request that fails
  before any reply or thinking text (connection refused/reset, no response headers,
  HTTP 5xx, or an Ollama `{"error":...}` frame in a 200 stream) retries automatically after 1 s, 2 s, 4 s… The session shows
  **Reconnecting in Ns · retry i/N**; Esc cancels immediately. Once any text has
  arrived, the partial reply is kept and retry stays manual (Ctrl+R). An error naming
  a missing model (`not found`), other 4xx, malformed frames, checks and tools are
  never retried.
- The header polls `GET /api/ps` every 5 s (2 s timeout, outside the queue):
  `ollama ✓ MODEL warm`, `ollama ✓ MODEL loading`, `ollama ✓ MODEL` (server up,
  model not loaded) or `ollama ✗ retrying`. Narrow terminals omit the model name.
  A restarted server is picked up without restarting Alfredo.

Queued cancellation removes eligibility before HTTP dispatch; process exit releases
its client slots. Saved queue records never replay inference. The ten-minute total
deadline still includes queue time; the loading/idle deadline starts after admission.
This coordination bounds Alfredo client requests, not GPU capacity or external
clients, and releasing a slot does not prove an active request stopped on the server.
No latency improvement or complete model/profile qualification is claimed.

Run `alfredo-tui --doctor` without a TTY to check startup prerequisites. It opens
and validates the selected workspace/mission/conversation storage, checks the
restored selected model against the bounded Ollama catalog, validates the Git root
and committed baseline, and checks installed worker executables. Exit 0 means these
checks passed; exit 2 means one or more checks failed. Invalid CLI arguments still
fail before diagnostics. Results distinguish storage, model server/catalog and
worker prerequisites with corrective flags or actions. A repository with no commits
fails the worker check with `git commit --allow-empty -m init`; the same notice shows
in the footer at launch, and `/go` there fails once without planning retries.

Diagnostics can initialize private state directories and lock files, but do not
save conversations, mutate task receipts, send inference, or run coding checks.
Model catalog membership does not prove model readiness or GPU capacity, and tool
presence does not prove sandbox permission. Actual worker execution remains subject
to the normal permission and sandbox checks. An occupied named conversation is
reported as a storage issue; choose another `--conversation` when appropriate.

A rejected slash command (unknown command, `Usage: …`, `Select a task first`, task not
found) keeps its text in the prompt so it can be corrected, but the text is marked
"replace on next keystroke": it is drawn reversed and the next typed character or paste
replaces it, so retyping the full command never concatenates (`/go/go …`). Left, Right,
Home, End, Backspace, Delete or Ctrl+W drop the mark and edit the text normally; Ctrl+U
clears it; Up recalls it from prompt history. An unknown command such as `/help`,
`/quit` or `/clear` answers `Unknown command /NAME · F1 lists commands · /go GOAL starts
autopilot`; `/help` itself opens the F1 command picker and `/model` without a name
answers `Usage: /model NAME (or /models)`. Footer notices wider than the terminal are
cut at a word boundary with `…`.

Use `/models` to load installed models from the configured server, then Up/Down and
Enter (or `/model NAME`) to select one for the current conversation. `▸` marks the
cursor and `›` the conversation's model. PageUp/PageDown scroll the catalog;
Escape closes it. Discovery runs asynchronously with a ten-second total deadline,
a 1 MiB response bound and at most 256 entries. Errors retain the previous catalog
and can be retried with `/models`. Listing a model does not prove it is loaded or
has sufficient memory to run. Active or interrupted turns retain their model;
finish/retry them or open another conversation to switch. Existing task assignments
remain fixed, while newly proposed tasks use the conversation's selected model.
See [Ollama model listing](https://docs.ollama.com/api/tags) for the provider protocol.

Launched inside a Git repository (from any subdirectory) with no workspace or
mission flags, Alfredo opens the repository root with mission `default`, resuming it
if it exists and creating it otherwise; no input is needed. `--workspace DIR` alone
does the same for the repository containing `DIR`. This uses the same validation,
admission journal and locks as the selector. If that automatic open fails (for
example, the conversation is open in another terminal, or mission state is
corrupt), the selector appears with the reason shown above its input.

Outside a repository, or with `--select`, launch starts with **Workspace selection
required**. The current directory is only a placeholder Starting Location; typing
replaces it. Enter validates an existing repository's exact root;
F2 selects new-repository creation at an unused path. The picker collects the
repository and mission choice without creating either. Existing files/directories
and nested repositories are never overwritten. After final mission confirmation
and saved admission, creation initializes Git with an
empty template and an empty initial commit, so planning and isolated workers have
a baseline. No project files are staged or created. Existing repositories are not
changed by selection.
Then type a mission name (placeholder `default`) and press Enter: an existing
mission resumes after identity validation, and a missing one is created. F2 is
optional and switches to **Start New Mission**, which refuses every existing name
with mission, task or conversation data. Validation errors appear on the line
directly above the input. Opening a mission grants no scope/task approval.
Esc exits the picker without creating a repository, mission or selection entry.
After confirmation, preparation reports repository, mission, target loading and
handoff separately; only the completed switch says **workspace selected**. A failed
creation retains any partial directory for inspection and does not automatically retry.
See [selection recovery](#selection-history-and-recovery) for the independent startup journal.

`--workspace DIR --mission NAME` resumes an existing mission directly. Use
`--workspace DIR --new-mission NAME` to create a distinct mission name. These flags
are mutually exclusive, and the repository is still validated. A mission flag without
`--workspace` opens the repository step of the selector. `--doctor`
remains noninteractive and uses current directory / `default` when flags are absent. `--state-dir DIR`
or `ALFREDO_STATE_DIR` selects task storage, defaulting to `$HOME/.local/state/alfredo`.
State must be outside the coding workspace. Each canonical workspace/mission pair
gets an independent `rust-tasks-v1/<sha256>/tasks.json`.

`--conversation NAME` selects a named conversation set within that workspace/mission
(default: `default`). Each set restores up to eight conversations. One terminal
owns a set at a time; use another name for a concurrent terminal sharing the task
queue. `--model` supplies the initial model for a new set; restored conversations
keep their saved models, which you can change using `/model` when eligible.

Snapshots save outside the workspace in `conversations-<name-sha256>.json`, beside
the task store. Changed snapshots are saved asynchronously about once a second,
with a final ordered save on normal exit. Abrupt termination can lose changes since
the last completed checkpoint. A saved active request restores as interrupted with
its saved partial answer; it is never automatically sent again. Corrupt, unsupported
or invalid state fails before terminal initialization and preserves the source file.
Conversation sets are bounded to eight sessions, 4,096 messages per session, 128 KiB
of message text and 16 KiB of draft per session, and 12 MiB of encoded JSON. Task/chat view, the last selected task ID and task search restore within the named conversation
set. A missing or filtered-out saved task has no shorthand action target; use
Up/Down or `/tasks #ID` to choose one explicitly. Group focus and collapsed branches
are local view state; restart expands the tree and restores the saved task anchor. Dispatch stays
off; evidence must be loaded again. Model catalog and transient progress are not
persisted. Version 1 snapshots migrate on save with an exact-byte `.v1-backup`
sibling; invalid state or conflicting backups stop the save.

Type `/task description` to propose a task, `/after 1,2 description` to propose
a task depending on existing earlier tasks, `/approve ID` to approve it, or
`/cancel-task ID` to cancel it. `/tasks` and F2 show the queue; `/chat` or F2 returns
to the conversation. `/refresh` reads the latest committed state. Errors preserve
the last acknowledged queue; `/retry-task` repeats the exact failed request.
Commands entered here are handled by Rust directly and never sent to a model.

`/activity` shows saved task receipts newest-first. `/activity words` searches
summaries, task titles, details and receipt IDs case-insensitively; `/activity #12`
filters exactly task 12. PageUp/PageDown scroll, `/refresh` reloads, and `/tasks`
returns to task details. The view records no navigation events and derives every
entry from the existing durable receipt ledger, including restored history and
idempotent retries. Entries show revision order rather than invented wall-clock
times or actor identities. It is not yet the full attributed Activity Journal.

The side pane's **work** section groups tasks under their recorded Plan (titled by
the user's goal; planner retry and revision text is cut off, also for older saved
plans) or **Manual tasks**, with repair descendants (`⑂`) nested beneath their
original task. A completed group starts collapsed while another group has open
work. Dependencies appear once, as `Depends` in the task detail. The section title
counts original tasks done; repairs are counted in the autopilot row. A recorded
run without a current observation is shown with a static `▶`, not a spinner: it
is not presented as a live worker. The architect appears above the tree while it
plans or holds a draft; chats appear below it.

The **missions** section lists the current mission first, then the other missions
of this repository discovered in the state directory. Their progress is
`done/total` task families and the phase saved in their autopilot state file
(`4/7   running`; phases `planning`, `running`, `paused`, `done`, `failed`), the
phase alone when the file has no counts (before a plan, or written by an older
build), `idle` when none exists, or `?` when unreadable; the files are read
without locks at most every 2 s, outside drawing. A running loop saves its counts
whenever one changes.

With task detail on the right, Up/Down selects a task or group. Alt+Left collapses
the current branch, or moves to its parent; Alt+Right expands it, or moves into its
first child. Ordinary Left/Right still edit the prompt. F6 focuses the side pane
(an overlay below 88 columns): Up/Down move within a section, Tab switches missions
and work, Enter opens the row, Alt+Left/Right fold, Esc (or F6) returns to the
prompt. While it has focus, typed characters and paste do not reach the prompt and
the draft is kept. Enter on a task opens its agent view, on a group its detail, on
the architect its agent view, on a chat that conversation; on another mission it
switches work with the same admission rules as `/workspace` (refused with the same
message while work is active).

### Agent view and owner instructions

The agent view (Enter on a task or the architect, or `/watch ID|architect`) is the
transcript of one agent: a task family (the task, its repairs and repairs of
repairs) or the architect. It is a projection only: the retained Local Agent
conversation (`agent-conversation.json`, verified against the evidence digest),
saved check evidence and the live worker observation. Turns, oldest first:
`You → worker #N` or `Autopilot → worker #N` (repairs: `→ repair #N`) with the task
title and `files … · check …` (Ctrl+O shows the retained request text),
`References` (read-only reference names from the request), `Worker` (the answer,
FILE blocks as `▸ path` then code, as in the live detail; markdown fence lines are
hidden, and inside a FILE block only the fence wrapping the whole file), `Check` (command, bounded
output tail, `✓ passed · exit 0` or `✗ failed · exit 1`), `Outcome`, then the next
attempt. Owner notes appear as `You` turns with their effect. A legacy, missing or
corrupt conversation shows the evidence with a one-line reason and invents nothing.
A steered or cancelled attempt keeps what streamed before the cut: its `Worker`
turn is that partial output, then a dim `— steered at 12s · output cut` (seconds
from model admission; `— steered · output cut` when unknown). The exchange is
retained marked `cut`; nothing is retained when nothing streamed. A repair after a
cut starts a fresh Local Agent and is never given the partial answer.
The title names the latest attempt (`Agent · worker #2 · running`,
`Agent · repair #3 of #2 · failed`, `Agent · architect · draft`). It follows the
tail while live; PageUp/PageDown keep the reading position like the chat.

While it is shown, the prompt title is `To worker #N · Enter send · Esc back` and
Enter records an owner instruction for the family's latest attempt. `/tell
ID|architect TEXT` does the same from any view. Each instruction chooses the next
command the owner could have typed and passes it through the saved-intent path, so
policy, evidence, locks and receipts stay authoritative and typed refusals apply:

| Latest attempt | Commands |
| --- | --- |
| generating (live worker, not in its check) | `/cancel-task N`; once the run is cancelled, `/repair N Owner: NOTE`, `/approve`, `/run` |
| running its check | none yet; on failure `/repair N Owner: NOTE · after check: FAILURE`, on pass dropped with `Note not needed: check passed` |
| failed, rejected, cancelled with a run | `/repair N Owner: NOTE`, `/approve`, `/run` (an unstarted autopilot repair of the same failure is cancelled first) |
| awaiting review | `/review N` needs-repair with reason `Owner: NOTE` (the existing review-and-repair transaction), `/approve`, `/run` |
| accepted | `/after ROOT NOTE` with the task's model, `/permit` with its exact files and check, `/approve`, `/run` |
| architect planning or draft | `/plan-revise NOTE` once the draft is complete |
| held for human review | refused: resolve it with `/review ID JSON` |

The task store cannot rerun a task after a cancelled run, so a steer is a repair
child of the cancelled run. Its parent was cancelled, not failed, so it is outside
the autopilot repair budget, and its request has no `WHAT IS STILL FAILING`
section. The worker request of an owner-instructed repair starts with
`OWNER INSTRUCTION (...)` and the note, above `WHAT IS STILL FAILING`. A note
approves the inherited policy only.

Unsent agent-view drafts are saved per task family and the architect in
`agent-drafts-SHA256(conversation).json` beside it (version 1, atomic replace,
at most 256 KiB, removed when empty) on close, every second, on mission switch and
on quit; the chat draft is saved separately and is never replaced by an agent's
note. An unreadable file is renamed `.json.corrupt` (then `.corrupt.1` …) with a
notice, and drafts start empty.

Instructions are saved in `owner-SHA256(conversation).json` beside the autopilot
state (version 1, atomic replace, the last 64 finished instructions kept for the
view) and continue after a restart by re-deriving each step from task state. While
an instruction is active its family is held from autopilot: autopilot makes no
decision for it, treats it as unsettled, and an autopilot command for it still
waiting at the saved-intent barrier is withdrawn (`Withdrawn: your instruction
decides for this task`). When both were prepared against the same revision, the
one that lands second is refused as stale and prepared again on current state.
Once the instructed run starts the hold ends and autopilot reviews it as usual. A
follow-up of a task in an autopilot run joins that run and appears in its plan
group in the work tree; a finished run reopens and integrates on
`alfredo/go-ID-2`, leaving the first branch unchanged. A follow-up that arrives
while the integration branch is being built discards that build when it finishes
(a branch it created stays, unused), then the run resumes and integrates on the
next `-N` branch.

A task row opens labeled sections: status and model, `Stage` while a worker runs,
`Files`, `Check`, `Depends`, `Repair`, `State` (readiness when it adds to the
status), `Next` (the command a decision needs), `Branch`, `Review`, `Criteria`,
then `Result`, the diff and check output. Long values wrap at word boundaries with
a hanging indent. A group opens its goal, progress and tasks and has no task
action target. An active filter is named in the detail title. Selecting a group retains
the last task ID for restart without allowing shorthand to act on that hidden task.
Selection stays anchored when background updates arrive. F3 opens selected-task
evidence. `/approve`, `/run`, `/cancel-task`,
`/evidence`, `/recover`, `/accept` and `/reject` may omit the ID to target the selected
task; a selected group refuses shorthand, and explicit IDs still work. Selection
waits while a storage request is pending. Navigation never approves or starts work.
PageUp/PageDown scroll details or evidence; changing rows returns to details and
resets its scroll position. At 32×10 the side pane and summary row give way to the
scrollable detail, with the prompt composer still reachable.

Task storage uses an exclusive OS file lock and atomic synced replacement. Every
change carries an expected revision and unique correlation id, with its action
and receipt committed together. Conflicting/stale requests fail without overwriting
state; exact retries return the stored receipt. Load validates the schema,
workspace/mission identity and bounded receipt replay against the task snapshot.
Limits are 256 tasks, 4,096 receipts and a 4 MiB store. Malformed state is preserved
for inspection. This protects consistency, not against a same-user attacker who
can coherently rewrite the whole private store. A bare approval grants no execution rights: workers require explicit policy followed
by approval. Schemas v1/v2/v3/v4/v5/v6/v7/v8/v9/v10/v11/v12/v13 remain readable; the first mutation saves exact original
bytes as `tasks-vN-backup.json` for the source version before publishing schema v14
in the same namespace. Legacy
approvals acquire no implicit permissions.

`/repair ID reason` proposes a linked repair for a failed, rejected or cancelled
run with verified retained evidence. It inherits the parent model, dependencies and
file/check policy, requires fresh `/approve NEW_ID`, and then `/run NEW_ID` starts
a new isolated worker. One unresolved repair per parent is allowed; exact request
replay returns the same child. The worker uses the parent's committed baseline and
includes the original task, outcome, patch and check receipt as bounded reference
data (128 KiB maximum evidence). It generates complete corrected files; the prior
patch is not executed or assumed applied. Original runs remain unchanged. Missing,
tampered or oversized evidence blocks the repair workflow; uncertain runs need
reconciliation first. Outside [autopilot](#autopilot), repair routing is manual; escalation remains unfinished.

While an approved check runs, the selected task shows live stdout and stderr.
Each stream retains an 8 KiB tail in memory, separate from task receipts. This is
best-effort progress: the shared provider uses a nonblocking 32-chunk observation
queue (at most 4 KiB per chunk), so a slow observer can miss chunks without delaying
capture, cancellation or timeout checks. The UI never interprets terminal controls
from this output. Live tails disappear when the worker finishes; `/evidence ID`
reads the saved bounded output and result. Output text alone cannot mark work done.

## Execute and review a task

Start in an existing Git repository with a commit, then enter:

```text
/task Create hello.py that prints Hello
/permit 1 {"files":["hello.py"],"check":["/usr/bin/python3","-B","hello.py"]}
/approve 1
/run 1
/evidence 1
/accept 1
```

`/permit` declares exact relative files and one check argv; changing policy resets
approval. `/run` claims a durable run and creates a detached worktree from committed
HEAD. Dirty working files are not copied. The model returns complete files as
FILE blocks (see "Worker answer format"), and Rust rejects unapproved paths before
writing. The approved check
runs in Bubblewrap with private network/process namespaces, system tools mounted
read-only, and only the isolated worktree writable. Host home directories and
external toolchains are unavailable. Git filters/includes are rejected at preflight.

`/evidence ID` opens a readable check summary, unified diff with signed/colored
additions and deletions, and separate saved stdout/stderr. It selects the inspected
task, so `/accept` or `/reject` without an ID acts on that evidence. PageUp/PageDown
scroll; the viewport clamps after resize or excessive scrolling. Parsing/rendering
never changes review authority; the store verifies evidence before display and
again before review.
`/accept ID` or `/reject ID` records review after checking evidence integrity.
Acceptance leaves the workspace branch and HEAD unchanged. Worktrees and bounded evidence remain
under the task namespace's `runs/` directory. `/cancel-task ID` requests cancellation
of a locally active worker and waits for its terminal receipt. Quit cancels active
workers and waits for their results. Up to four workers may run per terminal.

Successful worker results now retain a Git candidate commit for the exact reviewed
diff, parented by the recorded baseline. A `refs/alfredo/candidates/<commit>` ref
keeps the object reachable; the workspace branch, HEAD and working files stay
unchanged. Untracked check/build output is excluded. Evidence stores the optional
commit ID under its existing digest. Verification checks its sole parent and exact
binary diff against that evidence, without relying on mutable retained worktree
files. Older evidence remains readable with no inferred candidate. Snapshot
failure prevents a new result from becoming review-ready. Candidate refs are
retained; retirement and storage budgeting for them are not implemented yet.

For `/after 1,2 description`, each parent must be Accepted and retain a candidate
commit that verifies against its saved evidence. Before dispatch, the worker
combines those commits with the current committed workspace baseline using Git's
object-only merge-tree operation. Ancestor/diamond inputs are reused, not applied
twice. Conflicts, missing/tampered/legacy candidates and unaccepted parents leave
the child unstarted. Custom merge configuration requires qualification and is
refused before merge execution. Git must support `merge-tree --write-tree`.

The composed baseline is pinned under `refs/alfredo/bases/<commit>`. Preparation
can create immutable Git objects and managed refs, but moves no branch or working
files and starts no model/check. It has a 60-second deadline and observes worker
cancellation. The subsequent run claim records each exact parent task/run,
evidence digest and candidate ID; the store rechecks input identities and evidence
at that transaction. The child's isolated worktree starts from the composed
baseline, and its reviewed diff contains only its changes relative to that baseline.
The review view lists accepted input tasks. Local review branch handoff is available through `/branch` after acceptance.
Merging into the user's active branch, automatic conflict repair and managed-object
retirement remain separate unfinished workflows.


After accepting a result, `/branch ID` (or `/branch` for the selected task) creates
its local review branch, named `alfredo/task-ID-COMMITPREFIX`. The candidate must
still verify against saved evidence. The command creates the ref only when absent,
refuses a different existing target or symbolic ref, and records the verified
branch/commit in task activity. Repeating it verifies the same ref and avoids a
duplicate receipt, including reconciliation after Git succeeded but storage failed.
It leaves HEAD, the index and dirty working files unchanged. The result provides a
`git switch` command for deliberate checkout; no remote push or PR is performed.
Recorded branch links are historical; `/branch` rechecks the live target before
confirming it. Normal Git review/merge/push remains a separate user action.

Active workers show their current preparation/model/write/check/evidence stage,
time in stage, total elapsed time, received model bytes and first-content latency
measured from the model request. Timers refresh once a second even during model
silence. These are local observations; only the saved receipt establishes task
completion. Progress is not restored after a process restart.

Apart from autopilot's bounded repairs there is no automatic repair routing, worktree
retirement, aggregate disk budget or cross-process model capacity lease yet.

After restart, `/tasks` or `/refresh` distinguishes a live worker owner, a stopped
worker with saved final evidence or a proven check boundary, and an uncertain or
legacy run. `/recover ID` acknowledges valid final evidence or records a Failed
interruption under the [recorded check-boundary rules](#recovery-at-recorded-check-boundaries).
It refuses a live owner and never repeats inference, edits or checks. Repeated
recovery does not duplicate the result. Corrupt evidence and unproven outcomes
retain their original claim and artifacts; legacy claims without ownership markers
require inspection. Recovery does not establish that surviving child processes have
stopped or authorize worktree reuse or cleanup. Checks can leave untracked build
output in the retained worktree. Full runner recovery remains in the migration plan.

The crate is self-contained: the execution provider lives in `src/execution.rs`.

| Key | Action |
| --- | --- |
| Enter | Send current prompt (or fill draft when command picker is open) |
| F1 | Open command picker when draft is empty (`/help` opens it too) |
| Tab after `/prefix` | Open matching slash commands |
| Up / Down in conversation | Browse prompt history and restore unsent draft |
| First printable key after a rejected command | Replaces the rejected text (shown reversed); Left, Right, Home, End, Backspace, Delete and Ctrl+W keep it for editing; Up recalls it from history |
| Up / Down in task detail | Select a visible task or group |
| Alt+Left / Alt+Right | Collapse / expand a branch, or move to parent / child |
| F6 | Focus the side pane (overlay below 88 columns); F6 or Escape returns to the prompt |
| Enter in the agent view | Instruct that agent (text); slash commands still run |
| Escape in the agent view | Return to the previous pane; the agent's draft is kept, also across restarts |
| Ctrl+O in the agent view | Expand or collapse full instruction text |
| Up / Down, Tab, Enter in the side pane | Move, switch missions/work, open the row |
| Left / Right | Move through Unicode grapheme clusters |
| Home / End | Move to the start / end of the draft |
| Backspace / Delete | Delete the previous / next grapheme |
| Ctrl+W / Ctrl+U | Delete previous whitespace-delimited word / clear draft |
| Shift+Enter | Insert newline when supported by the terminal |
| Ctrl+N | Create another conversation (up to eight) |
| F2 | Switch the right pane between task detail and conversation |
| F3 | Open evidence for the selected task; groups require a task selection |
| Tab / Shift+Tab | Select next / previous conversation |
| Escape | Abort the selected client request and retain partial output |
| Ctrl+R | Explicitly retry a failed/cancelled turn, replacing its partial reply |
| PageUp / PageDown | Scroll transcript, task/group details or open evidence |
| Ctrl+Q / Ctrl+C | Quit and restore the terminal |

If a task write is pending, quit once waits for its receipt; pressing quit again
exits with an unknown save outcome. Read-only refresh does not block quit. Reopen
the queue to inspect committed state after an interrupted save.

Cancelling closes the client request; it does not claim that the server has
unloaded the model or stopped all inference. A request has a five-second connection
timeout, two-minute loading deadline (final, never retried: a retry would restart
the load), sixty-second idle timeout and ten-minute total deadline. Choosing another
model abandons the previous model's preload. Keyboard
input and rendering remain separate from inference. Completion requires Ollama's
`done` marker; EOF alone is a failure. There is no automatic replay. Drafts are
limited to 16 KiB, conversations to 128 KiB, frames to 64 KiB, and the event channel
to 128 entries. New sessions currently inherit the initial model. Draft editing preserves independent
cursor positions across sessions. Paste inserts at the cursor, strips terminal
control characters and respects the 16 KiB limit without cutting a grapheme cluster.
History is bounded to 100 entries and 128 KiB per session. Submission adds a history
entry without granting new authority; recalled commands require explicit submission.
Up/Down in task detail selects tree rows; Alt+Left/Right controls branch disclosure.
The command picker uses Up/Down or Tab to
select, Enter to fill the draft, and Escape to dismiss; a second Enter submits.
Autosave during history browsing preserves the original unsent draft/cursor.
Restored user prompts seed history; transient command history is not persisted.
The prompt viewport follows the cursor in terminal cells; newlines display as `↵`.
Traditional terminals may send Shift+Enter as plain Enter; pasted multiline text
remains supported.

## Verify

```bash
cargo fmt --manifest-path alfredo-tui/Cargo.toml -- --check
cargo test --locked --manifest-path alfredo-tui/Cargo.toml
cargo clippy --locked --manifest-path alfredo-tui/Cargo.toml --all-targets -- -D warnings
cargo build --locked --manifest-path alfredo-tui/Cargo.toml
python3 alfredo-tui/tests/terminal_smoke.py
```

Optional live check, using the installed `qwen2.5-coder:14b` model by default:

```bash
cargo test --locked --manifest-path alfredo-tui/Cargo.toml --test live -- --ignored --nocapture
```

The real coding smoke is `cargo test --locked --manifest-path alfredo-tui/Cargo.toml
--test worker live_local_model -- --ignored --nocapture` (Git/Bubblewrap required).

`ALFREDO_SMOKE_MODEL` selects a different installed local model. The check sends
one short prompt and prints observed first-content and completion timings; one
sample is not a performance benchmark. GitHub's `Rust terminal regression`
workflow runs deterministic gates and a release-build CLI smoke on Ubuntu.

Provider tests bind ephemeral loopback sockets. The Python test is a Linux PTY
acceptance harness only, not an application dependency. It drives the actual
binary against a local HTTP fixture, keeps one request stalled while another
completes, cancels, creates/approves a task, quits, reopens the same task namespace,
then executes an approved edit, inspects evidence, accepts it, verifies the original
workspace is unchanged, and checks terminal restoration plus conversation/model restart continuity. These deterministic
fixtures do not prove live-model speed or production launch readiness.

Protocol references: [Ollama chat](https://docs.ollama.com/api/chat),
[streaming](https://docs.ollama.com/api/streaming), and
[streaming errors](https://docs.ollama.com/api/errors), and
[structured outputs](https://docs.ollama.com/capabilities/structured-outputs). Rendering uses
[Ratatui](https://ratatui.rs/installation/), with its opt-in wrapped-line measurement
feature for transcript scrolling; the dependency resolution is committed in
`Cargo.lock`.

Task storage is bounded to 4096 command receipts and 4 MiB per mission. Commands
preserve space for running workers to record their results; a run cannot start
without completion capacity. A full journal refuses further commands. Archival
and capacity reclamation are not implemented. Older journals already lacking
space can acknowledge a saved result only if its actual receipt fits.

`/branch` checks request identity and journal capacity before creating its review
branch. A recorded handoff can be repeated at capacity without another receipt.
Concurrent journal writes or later disk failures can still leave a confirmed Git
ref awaiting its receipt; repeat `/branch` to reconcile the exact ref.

Use `/plan REQUEST` to generate a draft with the selected model acting as Frontier
Architect and assigning the same model to Local Agent tasks. The planner reads a bounded selection of committed repository files; review its
context/omissions, exact file paths, check argv, goals and numbered
dependencies before `/plan-save`. Saving creates all tasks as Proposed in one receipt,
with original prompt/planner provenance; it neither approves nor starts workers.
Use `/permit ID JSON` to correct policy, then `/approve ID` and `/run ID` as needed.
`/plan` reopens the current draft; `/plan-cancel` stops inference without task effects. Partial or invalid JSON cannot
be saved. Drafts are transient and are not restored after restart; saved plans are.
If task state changed during planning, saving refuses the stale revision and retains
the draft for inspection. Generate a new plan against refreshed state before saving.
Qualified profile/role registry, dynamic context retrieval, project-level Shared
Understanding and Plan Grill gates, and automatic dependency scheduling remain open.

`/models` lists installed choices. `/assign ID MODEL` changes an unstarted task's
Local Agent model, including tasks created by a plan or repair. The current catalog
must contain the requested model before a new assignment is saved. Exact replay
of an acknowledged assignment works offline. Assignment preserves policy and
blockers, resets approval, and requires `/approve ID` before `/run ID`. Running and
terminal tasks retain their model; create a repair task for another worker instead.
Model assignment and its receipt survive restart. Catalog presence does not prove
model quality, residency or capacity, and models may be removed after assignment.
The conversation model and historical Frontier Architect plan remain unchanged.

Planning context is pinned to committed HEAD. It includes a ranked file map (up to
256 paths / 16 KiB) and up to eight complete UTF-8 source blobs (8 KiB each, 24 KiB
combined, 64 KiB serialized context). Root instructions, README/context and build
manifests rank ahead of request-matching paths. At most 32 candidate blobs are read,
with a 30-second context deadline and the Git helper's 256-KiB output bound.
Symlinks/submodules, generated/vendor paths, common secret-file names, binary and
oversized sources are omitted. Counts and actual inputs appear in the preview and
saved Plan receipt; the selection is not a full repository audit or secret scanner.
Working edits and untracked files are excluded. A missing committed Git baseline or
context-read failure stops planning before inference. If HEAD changes before a
planned task starts, generate and review a fresh plan. Dynamic retrieval and planning
for uncommitted/new repositories remain open.

`/dispatch on` automatically starts ready Approved tasks in the current terminal.
A task needs an exact policy, no previous run and Accepted dependencies. Dispatch
shares the four-worker limit and model admission queue. `/dispatch off` stops new
starts; active workers continue until completion or `/cancel-task ID`. Shutdown
turns dispatch off before cancelling workers. Every new terminal starts with it off.

An approval is attempted at most once automatically during the terminal process,
including any earlier manual start. Failed starts remain visible in task details;
`/run ID` requests an explicit retry. A fresh approval permits another attempt.
Restart clears attempt memory but requires explicit `/dispatch on`; existing or
uncertain run records are never replayed. Dispatch does not approve tasks, accept
results, recover unknown effects or grant a cross-process inference lease.

`/tasks QUERY` finds tasks by title, worker model, status or readiness text (for
example, `/tasks blocked`). Matching tasks retain their Plan and repair ancestors
for context, and matching paths expand even through a collapsed branch. Counts
report matching tasks out of all canonical tasks, independently of ancestor rows.
`/tasks #12` selects and reveals the exact task ID; `/tasks` clears the filter.
Changing the filter preserves the last task anchor without selecting a substitute
when that task is hidden. Up/Down chooses a visible row; group rows and empty
results cannot target a hidden task. Opening `/evidence ID` clears the filter and selects
that inspected task for subsequent shorthand review. Search is a saved conversation-set view
preference and changes no receipts. Dispatch continues across the entire queue.
Task details distinguish missing policy, missing approval and each dependency that
is not yet Accepted; these explanations do not replace runtime launch validation.


Completion timing is shown as **Server timing** in the conversation, plan preview
and verified worker evidence when Ollama supplies it. Load, prompt evaluation,
generation and total durations are optional; generated tokens/s uses only reported
generation duration and token count. These are server observations, separate from
client queue and first-content latency. Invalid fields are ignored independently
(durations above ten minutes and counts above one million are omitted), and missing
metrics do not fail an otherwise complete response. Conversation and plan timing is
transient; worker timing is retained in digest-bound evidence. It grants no approval
or check success. See [Ollama chat response metrics](https://docs.ollama.com/api/chat).

New-repository preflight resolves existing runtime-path ancestors before testing
separation, so symlink aliases cannot hide runtime state beneath the future repository.
Dangling runtime symlinks, non-directory ancestors and unresolved parents refuse
before directory creation. This is a preflight check; later validation still applies.

At the mission step, **Saved mission names** lists names for the validated repository.
Tab cycles through and fills them; Enter opens the chosen name. You can always type
a different name. Discovery runs off the input thread, reads at most 1,024 namespace
entries and 16 MiB, and reports omitted/corrupt records. Names are sorted, not ranked
by recency, and carry no approval or task-status meaning.

Successful startup records an advisory `identity.json` beside the namespace's task
and conversation files. Existing task journals also supply name hints. Older
conversation-only namespaces become discoverable after opening them once by name;
their old hashed directory names cannot reveal the original mission name. A missing
or damaged discovery record does not erase conversations or task state: enter the
name manually, and normal loading still validates the selected state. Discovery
never follows state-directory symlinks or overwrites a conflicting identity file.

Worker ownership releases explicitly when its scope ends, including temporary
recovery probes. A fork-inherited descriptor cannot keep an orderly released claim
marked active. This changes no evidence rules: active owners still refuse recovery,
and missing or invalid retained results remain uncertain without effect replay.

The initial Open-repository step also lists **Saved repositories** from the same
bounded identity scan. Tab fills a saved path; Enter validates that exact repository
before listing its missions. Paths are sorted and deduplicated across missions,
not ranked by recency. Discovery does not check out, create or open a repository.
A moved/deleted repository can remain a hint; selecting it must pass current root
validation. In Create mode, Tab does not fill existing saved paths. You can still
type a path manually when discovery is incomplete or unavailable.

New mission admission writes a bounded, immutable `mission.json` under the namespace
lock before opening conversations. Concurrent Start New requests cannot both succeed.
Lost creation acknowledgment is resolved by explicitly resuming the same name;
retrying Start New never creates a duplicate or overwrites it. Resume also accepts
validated legacy task state or a valid legacy conversation snapshot, without rewriting
those files. An advisory `identity.json` alone cannot authorize Resume. Discovery
prefers the canonical mission identity where present. Invalid mission files are
preserved and refused. Scope agreement and Shared Understanding remain separate work.

## Shared Understanding (explicit native flow)

Use `/scope` to inspect the repository's saved understanding. To propose a draft,
review that state and submit `/scope` followed by JSON, for example:

```text
/scope {"destination":"A usable task dashboard","scope":"Terminal task supervision","constraints":"Preserve existing task receipts","uncertainty":"Keyboard accessibility still needs human review"}
```

Each field is required and bounded to 2 KiB. The draft becomes pending across all
missions sharing this workspace and native runtime. Review it and use
`/scope-confirm REVISION` with its displayed draft revision. Confirmation records
owner agreement only; it creates no plan, approval, agent or invocation.
A replacement draft requires fresh confirmation. `/scope-retry` repeats the exact
last scope write after an uncertain acknowledgment; `/tasks` returns to supervision.

While pending, the task store refuses new proposals, saved plans, policy/assignment/
approval changes, repairs and run claims. Model discussion and transient plan previews,
inspection, cancellation, completion of existing runs, review and accepted branch
handoff remain available. Exact acknowledged task requests can replay without effects.
This is a project-level check at the transaction boundary, not just a UI switch.
Absent native scope state means outside this explicit flow, never inferred confirmation.

The journal is bounded to 256 receipts / 1 MiB and reserves space for confirmation
before accepting each draft. Malformed, future or inconsistent state fails closed
for new governed work. This native journal does not import or govern older binaries' state. Automatic first-contact classification, planner
brief/provenance binding and full Wayfinder/mission-formation parity remain open.

Task details show the last observed scope state and include pending scope in task
search. Visible task views refresh once per second. A pending or unreadable scope
turns dispatch off; confirmation leaves it off until `/dispatch on` is requested.
A corrupt scope journal does not hide readable task history. Actual task writes and
worker admission continue to revalidate the saved gate independently of this display.

## Scope-bound native plans

The planner reads the workspace scope journal off the UI thread and supplies its
brief and revision to the model as reference data. The model cannot set provenance:
the adapter retains the captured binding alongside committed repository context in
the preview and saved Plan receipt. Activity shows the captured scope revision.
Pending-scope previews remain discussion only. Confirming or replacing their scope
requires generating a fresh plan before publication.

Task schema v9 adds optional Plan.scope (workspace, revision, draft revision, brief,
confirmed). Publication and planned worker owner/Start admission compare the complete
binding with current scope under the scope lock. A new agreement revision invalidates
an older unstarted plan even when its text is identical. Exact acknowledged requests
still replay without new effects. Existing runs can finish and be reviewed. Older
plans lack binding; they remain readable and usable outside the explicit scope flow,
but need replacement plans after that flow starts. No agreement is inferred during
migration; v1–v8 data gets an exact version-named backup before the first v9 mutation.
Manual tasks and repair proposals still have their own explicit policy/approval flow;
this binding does not yet provide scope provenance for every task-formation path.

Coding workers for saved plans receive that plan's captured scope binding in their
model request. The binding comes from the acknowledged task snapshot, so later scope
changes do not silently replace an already-started worker's context. The approved
file list and check argv remain the execution boundary; scope reference grants no
additional permission. Manual and repair tasks without a Plan retain their existing
context flow and do not acquire fabricated scope provenance.

## Native conversation client timing

The selected conversation shows a `Client` timing line. Queue time runs from submission
to the observed shared client-capacity admission event. First-text time runs from admission to the
first nonempty content event; streaming time runs from that event to the terminal
outcome. Total includes all phases. While waiting, the line shows admission pending or
waiting for text; no admission/first-text observation is fabricated on cancellation.
The active clock refreshes at most four times per second without incoming output.

These are monotonic UI-observed intervals, including event-delivery/render-loop delay,
not provider-internal measurements or a speed benchmark. Ollama's optional `Server
timing` remains separate. Completion, failure and cancellation freeze the intervals;
late events and prior retry attempts cannot alter them. A retry starts fresh clocks.
Timings are transient and excluded from saved conversations, so restart never revives
a stale running timer. Saved task/evidence schemas are unchanged.

Queue telemetry is also transient and attempt-bound. Its position includes the
current waiting request and excludes active slots; the total counts current waiting
requests. It is an observation, not a promised start time. Admission clears that
telemetry before upstream waiting. A cancelled or older attempt cannot change a
newer turn's queue, status or clocks.

The coordinator uses `/tmp/alfredo-inference-<effective-user-id>/<sha256-origin>/`
with private owner-checked directories/files, independent of `TMPDIR` and mission
state. Owner locks establish live eligibility; dead records are discarded without
replay. The bounded ledger holds at most 256 queued and active requests and 512 KiB.
Do not remove coordinator files while terminals are using them. Endpoint directories
are retained to avoid splitting a live lock; automatic namespace retirement remains
unfinished.

## Process-group cleanup identity

On Unix, cancellation cleanup retains an unreaped leader through the configured
SIGTERM grace interval. Its PID/start identity therefore remains verifiable before
forced group cleanup, even when the leader has exited and another group member
ignores SIGTERM. Existing identity/group checks and SIGTERM/SIGKILL paths are retained;
cleanup still requires a reaped leader and quiescent group before success. Failure to
prove cleanup remains outcome-unknown. This may use the full configured grace interval;
no timeout bounds or execution permissions are expanded.

## Thinking-stream progress

Ollama's [thinking protocol](https://docs.ollama.com/capabilities/thinking) can emit
reasoning separately from answer content. The native provider accepts thinking-only
frames and emits one payload-free progress event per request. Conversation status
shows `Thinking / waiting for text`, the planner shows `Model thinking`, and a coding
worker shows `Thinking` until answer content arrives. Reasoning text is neither
shown as an answer nor retained in conversation, plan or worker output. Thinking and
answer bytes share the existing 128-KiB output budget; individual frames remain
limited to 64 KiB. This preserves bounded transport while supporting thinking models.

Thinking status is transient, attempt-bound and cleared by retry/restart; late events
cannot revive completed or cancelled work. First-content timing still measures answer
text, not hidden reasoning. This change observes the phase without changing the
model's thinking configuration, token budget or execution authority.

## Native dependency advisory gate

Install pinned tooling with `cargo install cargo-audit --version 0.22.2 --locked`.
From `alfredo-tui`, run `cargo audit --file Cargo.lock --deny warnings --json`
with network access to refresh RustSec and registry data. The local `.cargo/audit.toml`
sets no advisory exclusions and takes precedence over personal audit configuration.
Do not add target filters or suppress warnings to obtain a green release gate.
From the repository root, `python3 alfredo-tui/tests/audit_smoke.py` then verifies
that the auditor rejects RUSTSEC-2022-0051 in a separate synthetic lockfile, using
the already-fetched database and no fixture compilation. The application lockfile
is untouched. CI retains the JSON report and runs this failure check.

On 2026-09-14, refreshing the database revealed RUSTSEC-2026-0285 in rustls 0.23.44;
the lockfile now uses patched 0.23.45. All 213 locked dependencies then audited
without findings or warnings. This point-in-time scan covers published advisories,
not undisclosed defects, application security or license compatibility. See the
[recorded before/after evidence](../../.agent/Reports/2026-09-14-native-dependency-audit.json).

## Installed-model completion

At the end of `/model PREFIX` or `/assign ID PREFIX`, press Tab to choose from
the installed model catalog. An empty prefix lists available names. Up/Down or
Tab cycles; Enter fills the draft and a second Enter submits the command. Esc
closes the picker without changing the draft. The task ID is retained. Completion
never selects a model, records assignment or grants approval on its own.

Use `/models` to refresh an empty or outdated catalog. Completion performs no network
request. Existing model-selection and assignment checks still apply at submission,
including restrictions on active/interrupted work and fresh approval after assignment.
Move the cursor to the end before completion; arbitrary command arguments keep
normal conversation switching. Installed names do not establish model qualification.

## Optional concurrent real-worker acceptance

With local Ollama and Bubblewrap available, run:

```bash
ALFREDO_SMOKE_MODEL=qwen3:14b cargo test --locked --manifest-path alfredo-tui/Cargo.toml --test worker live_parallel_workers_pass_independent_edge_case_checks -- --ignored --exact --nocapture
```

Two workers share one provider and independently edit temporary committed repositories.
One implements strict ASCII port parsing (valid boundaries and invalid types/text);
the other merges intervals across 3,375 combinations plus explicit edge cases, without
mutating inputs. Approved checks are outside model-writable files. Both must create
review-ready candidate commits while the source repositories remain unchanged.
`ALFREDO_SMOKE_PARALLEL_MODELS=1` overrides the test’s default two model slots
for controlled admission comparisons. `LIVE_WORKER_SAMPLE` lines record case, exact requested model, first-content/total
wall time, optional server metrics and outcome. This test uses synthetic fixtures
and is not a general model qualification or sustained workload benchmark. It stays
ignored in ordinary CI; run it explicitly against an installed local model.

## Generation limits

A response ending with Ollama `done_reason: "length"` is incomplete. The terminal
retains partial answer text and bounded server metrics, but marks the turn failed
instead of complete. Workers and planners cannot treat that response as successful
structured output, even if the partial text happens to be valid JSON. Shorten the
request or choose an appropriate model before explicitly retrying; no retry or
token-budget increase happens automatically. Normal `stop` and legacy responses
without a reason keep their existing completion behavior.

## Structured-request thinking

Planner and coding-worker requests now send `think: false` by default alongside
their JSON schema. This avoids a reproduced qwen3:14b/Ollama 0.34.0 failure where
thinking-only frames ended without answer text or a completion marker. The same
edge-case coding checks passed with thinking disabled. Ordinary chat requests
retain the model/server thinking default.

`--structured-thinking off|on|auto` explicitly sets this policy for schema-constrained
requests: off is the default, on requests thinking, and auto omits the option. Use
a mode supported by the selected model. The setting is invocation-local; pass it
again when restarting. It follows workspace switches within the process. No token
budget, deadline, admission limit or Ollama server configuration changes. No silent
retry or fallback occurs. This workaround is tested on synthetic coding cases;
complete role/model quality qualification remains open.

Coding workers in the default `blocks` format send no schema but keep this policy
(`think: false` by default) and the repair sampling temperature, so latency and
behavior stay comparable with the JSON request.

## Worker answer format

By default (`--worker-format blocks`) a coding worker answers in plain text:

```
=== FILE: relative/path.py ===
<complete file content, verbatim>
=== END FILE ===
```

One or more blocks, one per changed file; text outside blocks is ignored but kept
in the saved `model-response.txt`. Rules:

- Marker lines must start the line exactly (trailing spaces allowed).
- The path must be an allowed file; unapproved paths, duplicates, NUL bytes and
  the 32-file / 128 KiB bounds are refused as before.
- Content is taken verbatim; CRLF becomes LF and trailing newlines collapse to one.
  One markdown fence layer inside a block (first line starting with ```` ``` ````,
  last line ```` ``` ````) is stripped.
- Output ending inside a block fails with `Model output ended inside FILE block for
  PATH (truncated)`; no blocks fails with `Model returned no FILE blocks`. The next
  repair states that the previous response was truncated.
- File content cannot contain a line equal to a marker line. A FILE marker inside
  an open block is refused as ambiguous.

A legacy JSON answer (`{"files":[{"path","content"}]}`) is still accepted in either
mode, so older conversations and fixtures keep working. When continuing a Local
Agent conversation, a retained JSON answer is replayed as FILE blocks. A fresh
repair receives the previous attempt's files as FILE blocks, and prior evidence is
shown as plain text (patch and check output unescaped).

`--worker-format json` sends the legacy schema-constrained JSON request instead.
Evidence records the requested format in `generation.answer_format`; older
evidence has no field (JSON request, unrecorded). Inference qualification always
pins `json` so recorded request profiles stay comparable.

## Requested generation settings in worker evidence

New worker evidence records `generation` when preparing a schema-constrained model
request: requested thinking mode (`auto`, `on` or `off`), `num_predict` and temperature.
The verified evidence view shows these values. The record describes configuration;
it does not prove HTTP dispatch, server compliance or model qualification. Failures
before model-request preparation can have no generation record.

The field is optional and included in the existing evidence-byte digest. Legacy
evidence stays unchanged and displays “Requested generation: unrecorded”; no default
is inferred for past runs. Unknown thinking modes, unknown metadata fields and
out-of-bound numeric values reject deserialization. `auto` is an explicit recorded
choice, distinct from absent legacy data. This additive evidence field does not
change task receipts, scopes, permissions or snapshot schema versions.

## Refine an unsaved task plan

After `/plan REQUEST` completes, use `/plan-revise REQUEST` to refine its tasks,
paths, checks and dependencies. The planner receives the previous task list,
original request and accumulated revision requests, plus freshly captured committed
repository context and project scope. It retains the draft's planner model even
if the conversation model has changed. Review the complete replacement before
`/plan-save`; saving proposes tasks and never approves or runs them.

A failed or malformed revision restores the previous complete draft with its
original task revision, so a failed refinement cannot refresh a stale save. Saving
and another revision are blocked while inference is active. `/plan-cancel` discards
the current draft and pending revision; late responses cannot restore them. Revision
requests share the 8 KiB prompt limit, and previous task reference data is bounded
to 64 KiB. Complete drafts now survive restart as described below; full Mission
Draft/Issue Graph formation remains unfinished.

## Plan draft restart continuity

Conversation schema v5 introduced retention of a complete task-plan draft and its original
task revision. Autosave, normal shutdown and a quiescent workspace/mission switch
preserve it under the existing conversation owner lock and atomic save. A pending
refinement checkpoints its prior complete draft; partial first-generation output
is not a plan and is not restored. No inference resumes automatically.

Restored plans open for review, retaining their original repository/scope bindings
and stale-state checks. `/plan-save` still requires explicit submission and approval
remains separate. `/plan-cancel` clears the draft on the next checkpoint or normal
shutdown. A completed save followed by a crash before conversation checkpoint may
restore an older preview, but its old task revision prevents duplicate publication.
Saved plans are limited to 256 KiB and must pass the normal plan validation.

V1–v4 snapshots remain readable. First v5 save keeps the exact source bytes in a
versioned `.vN-backup`; conflicting backups, future versions, invalid plans and old
versions carrying plan data refuse without overwriting the original. Those v5 continuity changes did not alter task/scope schemas; see the current
acceptance-criteria migration below. Full Mission Draft/Issue Graph formation is still open.

## Explicit Wayfinder capability

Type `@wayfinder REQUEST` to address the native scope adapter. Tab completes the
name; arrows select, Enter fills the composer, and Escape dismisses completion.
Completion never submits a turn. F1 lists this capability alongside commands.
Unknown leading `@` names and an empty Wayfinder request produce an error and retain
the draft without model dispatch. Mentions inside ordinary prose are not commands.

An explicit discussion request without a saved scope enters Chart (or Work-through
for an existing Wayfinder map/ticket). Ordinary read-only prompts outside the capability keep their existing
exclusion from automatic Chart. Four-field briefs and exact-revision confirmation
also accept the prefix; their outcomes retain the same scope receipts and grant no
task approval or execution. Subsequent discussion reuses the existing flow.
Other native roles remain accessible through their documented commands; this is
not a general skill/plugin executor or complete capability-routing implementation.

## Recovery at recorded check boundaries

When a worker stops, inspect its task and use `/recover ID` explicitly. Recovery
requires the stopped worker's owner lock. Valid saved final evidence takes
precedence and retains its recorded outcome, even if a check checkpoint is damaged.
Existing malformed final evidence is preserved and blocks recovery.

Without final evidence, recovery can record **Failed** at either proven boundary:

- Before check launch: the version-1 `execution-boundary.json` matches the task,
  run and baseline, and both check intent and result are absent.
- After a recorded terminal check: the version-2 `check-launch-intent.json` binds
  the exact authorized request and canonical digest, Mission, task, run and
  baseline. Contract version 1 fixes the worktree, approved files/check argv,
  recorded system mounts, environment and resource policy. The version-1
  `check-result.json` binds the exact intent bytes and request digest to the
  provider receipt, including bounded output, byte counts, hashes and identities.

The worker creates each artifact exclusively and syncs the file and parent
directory. Intent publication finishes before check launch; result publication
finishes inside the execution closure before worker finalization continues.
Publication failure prevents a success claim. These immutable records live outside
the worker-mounted worktree. Digests detect corruption and substitution; they do
not authenticate private state against a same-user actor replacing all records.

After-check recovery retains the original check receipt and output and reports
**interrupted after check; candidate not finalized**. Even a zero-exit check yields
a Failed task: recovery reconstructs neither patch nor candidate. Original partial
work and artifacts remain intact. Repeating recovery returns the same Finish
acknowledgment without repeating an effect. `/repair` proposes separate work that
requires fresh approval; dependencies remain blocked on the failed original.

An old unbound check intent, missing or partial result after launch, unsupported or
corrupt artifacts, mismatched identities, and uncertain or reconciliation-required
receipts grant no after-check recovery. Existing runs gain no proof by default.
These separately versioned artifacts require no task, conversation or scope schema
migration. Recovery invokes no Git, inference or check, respawns no worker, and
does not signal surviving processes. Owner release and a terminal check receipt do
not establish quiescence of later worker helpers or authorize worktree reuse,
retirement or cleanup. Full automatic runner recovery remains separate work.


Task-panel navigation: PageUp/PageDown uses the current panel's visible height,
retaining one row of overlap when possible and advancing at least one row in tiny
panes. Paging clamps at the displayed content boundary, including after resize,
so narrow inspectors do not skip rows. Task details, scope, Activity, plan and
evidence panels share logical-line slicing so multi-line content can extend beyond
65,535 rows.
Activity and Models use the focused compact layout at narrow terminal widths,
keeping their content and the composer reachable at the 32×10 minimum size.


Evidence rendering retains one width-specific wrapping index for the immutable
verified view and copies only the visible logical lines on redraw. Resizing the
width or replacing the evidence rebuilds the index; height changes reuse it. This
cache is transient presentation state and adds no persistence schema or authority.

The explicit synthetic rendering measurement runs without a timing pass/fail
threshold (machine load varies):

```bash
cargo test --release --locked --offline --manifest-path alfredo-tui/Cargo.toml \
  --test review retained_evidence_render_measurement -- --ignored --exact --nocapture
```

It renders retained 66,000-line evidence while editing the composer, reporting
cold-draw and 100-redraw time. It does not measure model or end-to-end latency.


## Explicit plan acceptance criteria

New generated plans require 1–16 distinct observable acceptance criteria per task,
each a nonempty single line of at most 1024 UTF-8 bytes. Review them alongside the
paths, check command and dependencies before `/plan-save`. Saving still creates
Proposed tasks; approval remains separate. A passing check does not automatically
establish every criterion or accept a task.

Task schema v10 retains the criteria in the immutable Plan receipt. The selected
task and evidence review show the recorded contract, and workers receive it as
reference within the approved policy. Repair descendants inherit the original
criteria and need fresh approval. Legacy/manual tasks without criteria explicitly
say not recorded; no criteria are inferred from a command or successful result.

Conversation schema v6 preserves criteria in unsaved plan drafts across restart
and quiescent mission handoff. Task v1–v9 and conversation v1–v5 remain readable;
the first newer write retains an exact version-named backup. Conflicting backups,
malformed criteria, future schemas and older schemas claiming new criteria refuse
without replacing the original. Task/conversation locations are unchanged.

This advances the Local Agent task-packet contract. Full Mission Draft/Issue Graph
formation and attributed action chronology
remain separate unfinished requirements.


## Criterion-level review

Use `/review ID JSON` to record an explicit accept/reject decision, its reason and
an evidence note for each recorded criterion, in order starting at 1:

```text
/review 4 {"accept":true,"reason":"Reviewed implementation and checks","criteria":[{"criterion":1,"met":true,"note":"Retained test asserts VALUE equals 42"}]}
```

Acceptance requires every recorded criterion to be marked met. A rejected review
may mark criteria not met. Notes are reviewer assertions supported by the inspected
evidence, not independently verified facts. The reason is a nonempty single line
of at most 2048 UTF-8 bytes; each note is a nonempty single line of at most 1024
bytes, with at most 16 ordered criteria. Unknown fields and mismatched coverage
refuse. Legacy/manual tasks without recorded criteria use an empty criteria list.

New `/accept` calls for tasks with criteria refuse and direct the user to `/review`.
`/accept` for tasks without criteria and `/reject` remain available. Historical
boolean review receipts remain readable and exactly replayable without invented
reasons or criterion assessments. Both review paths retain the existing successful
check, evidence-digest, expected-revision and exact-correlation guards. No inference
or repair runs inside the review transaction. Enabled dispatch may subsequently
start already-approved dependents when their parent becomes Accepted.

Task schema v11 adds the Assess receipt. Replay validates each assessment against
the already validated Plan receipt prefix and repair lineage. Before the first v11
mutation, a v1–v10 store receives an exact version-named backup; conflicting backups
or older schemas carrying Assess receipts refuse unchanged. Conversation v6 and
scope v2 are unchanged. Notes appear in task details, saved Activity and evidence
review; an open evidence view updates after acknowledgment. Rejected notes become
reference data for a separately proposed and approved repair, under the existing
128-KiB combined repair-context bound.

Explicit outcome support is documented below. Automated Frontier Reviewer
decisions and tiered automatic repair routing remain unfinished.

## Explicit review outcomes (task schema v12)

`/review ID JSON` also accepts `outcome` in place of the legacy `accept` field:
`approved`, `approved-with-limitations`, `needs-repair`, `needs-human-review`, or
`rejected`. The reason and ordered criterion evidence notes remain required.

```text
/review 4 {"outcome":"approved-with-limitations","reason":"Inspected implementation and checks","criteria":[{"criterion":1,"met":true,"note":"Retained check verifies VALUE equals 42"}],"limitations":["Performance outside this fixture remains unmeasured"]}
```

Both approving outcomes require all recorded criteria met, intact evidence and an
original successful worker completion. Limited approval additionally requires
1–8 distinct, nonempty single-line limitations, each at most 1024 UTF-8 bytes.
Other outcomes cannot carry limitations. Limitations cannot waive failed criteria.

Needs human review holds the task and its dependents. Direct approval, run and
repair cannot bypass the hold; an explicit new review decision must resolve it.
A failed run held for review still cannot be approved. Needs repair and Rejected
remain unaccepted; repair requires a separate proposal and fresh approval.
Saved Activity, task details and open evidence show the recorded outcome; resolving
a hold replaces stale review notes while preserving criteria and dependency inputs.

Task schema v12 adds the Decide receipt and human-review status. The first mutation
of a v1–v11 store preserves an exact version-named backup; conflicting backups and
older schemas carrying Decide refuse unchanged. Legacy Review and Assess receipts
remain readable and replayable. Conversation v6 and scope v2 are unchanged.
Automatic reviewer inference, tiered repair routing and architect escalation remain
unfinished; these decisions are explicit user actions.

### Human-review repair exclusivity

A repair child awaiting human review counts as unresolved. New `/repair` proposals
from its parent refuse until that hold is explicitly resolved. This guard applies
under the task transaction lock, including after restart, without changing task
schema v12. Older v12 receipts that already created a sibling remain readable and
exactly replayable; the application does not discard or reinterpret saved work.
Resolving the held child's review can then support a separately proposed repair
with fresh approval.

## Review risk escalation (task schema v13)

A review may declare `risk` as `critical`, `security`, or `merge-risk`:

```text
/review 4 {"outcome":"rejected","risk":"security","reason":"Review found an unsafe input path","criteria":[{"criterion":1,"met":false,"note":"Inspected input handling needs correction"}]}
```

Recording any of these risks with Rejected, Needs repair or Needs human review
automatically holds the task for human review. The original outcome and risk remain
in the receipt and Activity; task status shows the hold. This blocks dependent
execution, direct approval/run, repair of the held task and sibling repair proposals
from its parent. Risk classification is a reviewer assertion, not automatic risk
detection. Absent classification means unrecorded, not verified safe.

Approved outcomes carrying a risk refuse. Resolve a held task with a subsequent
explicit `/review`, omitting `risk` and explaining the human decision in `reason`.
Approval still requires the original successful check and every recorded criterion
met. A failed run cannot become approved through risk escalation. Resolving to
Needs repair permits a separate repair proposal that still requires fresh approval.
No model request or repair starts inside the review transaction.

Task schema v13 adds the optional typed risk. Exact v1–v12 backups precede the first
mutation; older schemas carrying a classified risk refuse unchanged. Existing
unclassified reviews remain readable/replayable with no invented classification.
Conversation v6 and scope v2 are unchanged. Same/fresh-agent continuity and
Architect revision routing remain unfinished.

## Local Agent repair continuity

Each newly requested worker conversation records a mission-local Local Agent
identity in its evidence. After a complete model response, Alfredo retains the
exact user/assistant exchange in `agent-conversation.json` beside the run evidence,
with a SHA-256 reference in that evidence. This is application-managed conversation
continuity through ordinary chat messages; it does not depend on hidden server
memory or claim reuse of an inference process.

On `/repair`, the first rejection can continue the prior Local Agent: the next
request includes its retained user/assistant messages followed by the current
repair prompt. The second or later terminal rejection in the repair ancestry
starts a fresh Local Agent and sends only the current prompt plus verified repair
evidence. Needs repair reviews do not themselves count as rejections. Every repair
still requires separate approval and uses the original baseline with current exact
file/check permissions. Historical messages grant no additional permissions, and
prior patches are not automatically applied.

A changed model, legacy run without recorded conversation, or incomplete prior
model exchange starts a fresh conversation with an explicit reason. Referenced
history needed for continuation is checked for regular-file identity, bounds,
hash, run/model/agent binding and alternating roles before claiming the repair run.
Missing or damaged referenced history refuses; it is not silently replaced.

History is bounded to eight messages and 512 KiB of content, reserving the existing
128-KiB response budget before each request. Reaching the history budget explicitly
starts a fresh Local Agent while preserving the current repair prompt. Serialized
conversation files are limited to 4 MiB and written exclusively and durably. The
original evidence/context/check limits remain in force. A disconnect before model
completion does not create a complete retained exchange or replay effects.

Evidence review shows fresh/continued identity and the reason. Legacy evidence
shows conversation unrecorded. Task schema v13, conversation-set schema v6 and
scope v2 remain unchanged; the separately retained agent transcript has schema v1.
Review-triggered automatic repair proposal/launch, Architect revision routing,
qualified model selection and retention/storage lifecycle remain unfinished.

## Atomic review-to-repair proposals (task schema v14)

`/review ID JSON` now records an unclassified `needs-repair` or `rejected` outcome
and creates an inherited repair task in one transaction. The terminal names both
the reviewed parent and the new child, and links the child from parent readiness.
The child is Proposed: inspect it and explicitly `/approve CHILD` before `/run` or
enabled dispatch can start it. Recording the review does not launch a model/check.

```text
/review 4 {"outcome":"needs-repair","reason":"Correct the calculation edge case","criteria":[{"criterion":1,"met":false,"note":"Observed failing boundary input"}]}
```

The compound receipt stores both review intent and the child proposal; its primary
receipt task is the new child. Activity can find it under either parent or child.
Review notes and acceptance criteria follow repair lineage, and Local Agent
continuity counts compound Rejected decisions exactly like earlier rejections.
Risk-bearing reviews still enter a human hold without creating a repair.

Evidence, criterion coverage, open project scope, task/receipt/storage capacity and
unresolved-child guards apply to the whole operation. Failure leaves both review
and proposal uncommitted. Exact retries after restart return the same child;
conflicting/stale requests cannot add another child or overwrite the outcome.
A pending scope gate blocks the compound operation because it proposes new work.

Schema v14 adds `review-and-repair`; earlier schemas cannot contain that receipt.
Exact v1–v13 backups precede migration. Old Decide, Assess and boolean Review
receipts retain their original meaning; they do not retroactively create children.
Legacy `accept` JSON and `/reject` remain review-only, with `/repair` available for
explicit separate proposals. Conversation v6, scope v2 and agent transcript v1 are
unchanged.

Repair dependency resolution is implemented in schema v15 below. Review-triggered
execution authorization and full launch qualification remain open.

### Resolve an accepted repair

After accepting a repair, use `/resolve-repair ID` to select it as the result for
its unsuccessful ancestors. Future consumers keep their declared dependency IDs
and record the actual accepted repair source. Existing runs retain their inputs.
The original tasks retain their outcomes; resolution does not mark them accepted.
Unresolved sibling work and human holds block resolution. The selection is final
for that repair family; new repairs or reviews cannot reopen it. Repairs reuse
exactly their parent's baseline and recorded inputs. Git candidates are verified
again before dependent execution.


## Architect revision after repeated failures

A reviewer can set `"failure":"architecture"` on a nonapproval `/review` decision.
The first classified failure proposes a normal repair. A second distinct reviewed
run in that lineage records an Architect route and stops ordinary repair work;
critical/security/merge risk takes precedence and remains a human hold. Absence
of classification stays unrecorded, and repeating a review cannot manufacture a
second failed run.

The current acknowledgment opens the real Frontier Architect when no draft or
inference is already active. `/architect-revise ID` explicitly resumes a pending
route after disconnect/restart or after another draft is cleared. Restoring state
never repeats inference. The Architect receives bounded verified lineage evidence,
review notes and criteria, plus repository context from the parent's exact commit.
It produces one revised repair task with explicit criteria and exact file/check
policy. This bounded repair revision does not rewrite unrelated mission tasks.

`/plan-save` explicitly adopts that draft as a linked Proposed repair, preserving
original dependencies and execution baseline/inputs. New paths/checks still require
fresh approval. The revised task starts a fresh Local Agent conversation and a new
architecture-failure cycle. Accepted revised work uses `/resolve-repair` normally.
Source task/run/evidence digest and route revision bind the draft; stale or tampered
source evidence refuses adoption. Draft refinement/restoration retains provenance.
Task schema16 and conversation schema7 introduce these fields with exact prior
version backups; earlier schemas cannot claim the new provenance.

The latest adopted Architect repair defines the active family branch. Older branches
cannot start workers, create repairs, change reviews or resolve the family. Cancelling
an adopted repair before it runs reopens its source Architect route; an explicit
new draft/adoption can replace the cancelled proposal without restoring old policy.


### Follow background coding work

The header keeps running local workers, pending reviews and decisions visible while
you chat (each only when non-zero), and the side pane keeps the work tree beside
the conversation. F4 opens saved task Activity; F2 returns to the conversation without
clearing your draft or moving its reading position. A “recorded run” means the
saved task is Running but this terminal does not own its worker; inspect its
retained evidence rather than assuming it is active or safe to replay.

The chat console now shows **Observed task receipt** entries for newly observed task acknowledgments, including exact revision/correlation and phase. These are local observation positions, not claims about who requested an action. Receipts themselves are not sent; instead each ordinary chat turn (outside Wayfinder) starts with a system message naming Alfredo's role and this mission's task records: goal, status, files, check, outcome and verified patch (newest first, 3 KiB per patch, 16 KiB total). The records are reference data, not instructions, and are not saved in the conversation. Old receipts stay in F4 Activity; saved observation references survive restart. Receipt updates wait for an active selected model turn to finish so streaming does not insert blocks into the reading position.

Task mutations, scope changes, explicit worker starts, branch creation and recovery now save their immutable command intent before dispatch. The console keeps the command and matching receipt in its originating session. Interrupted commands restore as unconfirmed without automatic replay. `/retry-command SESSION:COMMAND` explicitly retries a saved command number, including from another session; the exact intent and origin remain unchanged. `/retry-task` can use the selected session's latest unresolved saved command after restart. Planner generation and worker controls also use saved intent, with their separate outcomes described below.

### Saved planner commands

`/plan REQUEST`, `/plan-revise REQUEST`, `/architect-revise ID`, and `/plan-cancel`
are saved before dispatch. Their outcomes remain in the originating conversation.
Generating a draft does not save or approve tasks: use `/plan-save`, then review
and approve the proposed tasks separately. Revisions bind the exact previous draft.
Cancelling active revision retains its previous draft; cancelling an idle draft
discards it. If the target changes while the command is being saved, cancellation
refuses and you can inspect the current plan before trying again.

Restart never resumes inference. A retained draft can restore its exact generating
command's result. An interrupted command without that proof remains unconfirmed;
completed planner commands require a new command to generate again. Conversation
schema12 preserves an exact backup when upgrading older snapshots.

### Saved worker controls

`/cancel-task ID` for an active worker records the exact controller and worker
identity before requesting cancellation. The command shows **Cancellation requested**;
the separate worker-result line comes from its canonical Finish receipt. A worker
that finished before the request took effect can still report a successful result.
A stale request never targets a replacement worker.

`/dispatch on` and `/dispatch off` are also saved before application. Enabling checks
the current scope asynchronously; disabling invalidates a pending enable and leaves
active workers running. The header shows `dispatch on` while dispatch is on and
nothing while it is off. Saved outcomes describe the originating controller; restart starts dispatch
off and never replays controller requests. Conversation schema13 preserves exact
older snapshots during migration.

### Automatic launch history

Each automatically selected worker is saved as a **Dispatch** launch in the same
conversation as its enabling `/dispatch on` command. Selection captures its exact
approval and current task revision; the worker starts only after that launch entry
is saved. Its Start and Finish remain separate phases. Background entries preserve
your current conversation, reading position and unfinished prompt.

A failed save or stale launch stops dispatch and reports the refusal. Use an explicit
`/run ID` after inspecting the task; historical automatic entries are never replayed.
`/dispatch off` can withdraw a launch still awaiting its save acknowledgment.
Restart retains the history, starts dispatch off, and launches no worker automatically.
Conversation schema14 introduced this source link; schema15 added automatic
Architect provenance, and the current schema17 retains both. Exact older snapshots are preserved
before migration.


### Automatic Architect history

A review-triggered Architect revision is saved as an **Architect** draft entry in
the review command's conversation. It identifies its source review and preserves
your selected conversation, reading position and unfinished prompt. The draft
starts only after its exact entry is saved. Generation does not save or approve
tasks; inspect the draft and use `/plan-save` separately. Restart never resumes
inference, and a retained completed draft can reconcile only its exact origin.

Conversation schema15 preserves exact prior bytes and rejects missing, mismatched
or duplicated planner origins. Task schema16 remains unchanged. `/plan-cancel`
can withdraw an automatic draft still awaiting its intent save. Once a shared model
slot is available, the planner rechecks the captured task revision and Architect
origin before sending the model request.


### Saved Wayfinder scope actions

Wayfinder entry, four-field drafts and exact-revision confirmation now keep a
compact scope action beside the originating conversation turn. The action saves
before scope changes and displays only its exact scope receipt as acknowledged.
Your original prompt remains a normal conversation message; the action does not
repeat it or add controller metadata to model input.

Cancelling a reply or switching sessions does not discard an already submitted
scope action. Its receipt stays with the original turn. Restart keeps unresolved
actions unconfirmed and never replays them automatically; an explicit retry keeps
the same request. Confirming Shared Understanding still approves no tasks.
Conversation schema16 adds this turn binding and preserves exact prior snapshots.
Full mission formation and production launch acceptance remain unfinished.

### Selection history and recovery

Workspace and mission choices save before creation or handoff. An in-process
**You · selection #N** entry stays in its originating conversation; a matching
**Workspace · arrival #N** entry preserves the destination's existing messages,
draft and reading position. These observations do not approve tasks. **Handoff
prepared** means the destination history was saved, while **workspace selected**
records the later switch. Historical selection does not change the current header.

Startup has no originating conversation. Its exact request and observed phase are
stored in `<state-dir>/rust-selection-v1/selections.json`, also used by in-process
selection. The default is `$HOME/.local/state/alfredo/rust-selection-v1/selections.json`.
After a failed launch, inspect that file's matching `request` and `outcome`, the
requested repository path and any retained mission state. Keep the journal and
partial artifacts; a pending or incomplete record does not prove no effects occurred.
If the repository exists, choose **Open** and explicitly **Resume** the saved mission
after inspection, or **Start New** with a fresh mission name when appropriate.
Never use another Create request to overwrite a partial directory. A malformed
journal is preserved and refuses further selection; restore a known-good state
backup after stopping terminals rather than deleting history to force a retry.

Conversation schema17 preserves these exact source/arrival bindings and older
snapshots. Restart reconciles matching journal observations without replaying
creation or selection; unavailable proof remains **Outcome unconfirmed**. Task
schema16 and scope schema2 are unchanged. Full mission formation, execution recovery,
retirement, platform qualification and production launch acceptance remain open.
