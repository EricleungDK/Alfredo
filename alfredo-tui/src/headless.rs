//! `--go GOAL --headless`: the autopilot without a terminal. Builds the same
//! workstation the TUI uses and drives the same task-control, autopilot and
//! saved-intent barrier in its own loop; progress goes to stderr, the result is
//! one JSON object on stdout and the exit code tells the outcome (0 all accepted,
//! 1 partial/failed/interrupted; setup and usage errors are the caller's 2).
//! `--chat PROMPT --headless` ([`run_chat`]) runs one Chat Agent turn the same way;
//! its Approval gate is answered by a scripted [`Policy`], never by a skip mode.
use alfredo_tui::{
    autopilot::{RunState, Status, TaskOutcome},
    chat_tools::approval::{Card, CardKind, Decision, Gate, Log, Outcome, Record},
    chat_tools::{MAX_STEPS, STEP_CAP_NOTICE},
    command_intent::Intent,
    console_command::{bounded_reason, CommandState},
    model::{Stop, Update},
    provider::Ollama,
    tasks::TaskStatus,
    workstation::Workstation,
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Component, PathBuf},
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

static INTERRUPTED: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_signal(_: libc::c_int) {
    INTERRUPTED.fetch_add(1, Ordering::SeqCst);
}

/// `--output-format stream-json`: stdout carries one JSON event per line.
static STREAM: AtomicBool = AtomicBool::new(false);

/// Version of the stream-json event schema (`"v"` on every line).
const STREAM_VERSION: u32 = 1;

/// Turn the stream on or off for this process (`--output-format`).
pub fn set_stream(on: bool) {
    STREAM.store(on, Ordering::SeqCst);
}

/// One stream-json line, `{"v":1,"type":kind,…fields}`; a no-op without the flag.
/// A closed pipe must not abort the run.
fn emit(kind: &str, fields: serde_json::Value) {
    if !STREAM.load(Ordering::SeqCst) {
        return;
    }
    let mut event = fields;
    event["v"] = STREAM_VERSION.into();
    event["type"] = kind.into();
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{event}");
    let _ = out.flush();
}

/// The final result object: a plain line, or the stream's last `result` event.
fn print_result(result: serde_json::Value) {
    if STREAM.load(Ordering::SeqCst) {
        emit("result", result);
    } else {
        let _ = writeln!(std::io::stdout(), "{result}");
        let _ = std::io::stdout().flush();
    }
}

/// Progress line on stderr; a closed pipe must not abort the run.
fn say(line: impl std::fmt::Display) {
    let _ = writeln!(std::io::stderr(), "{}", stderr_line(line));
}

/// One printable line for stderr. Approval cards, tool names, paths and errors carry
/// model-chosen text, so control characters (ESC, BEL, C1 CSI, CR) go, format characters
/// such as bidi overrides show as `<U+XXXX>`, and line breaks (including U+2028 and
/// U+2029) become spaces: the terminal or CI log shows text, and no second line can be forged.
fn stderr_line(line: impl std::fmt::Display) -> String {
    alfredo_tui::dashboard::single_line_visible(&line.to_string())
}

/// SIGINT and SIGTERM request the same cancellation as `/stop`.
fn catch_signals() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

pub struct Options<'a> {
    pub runtime: &'a Runtime,
    pub provider: Ollama,
    pub state_dir: PathBuf,
    pub start: PathBuf,
    pub conversation: String,
    pub model: String,
    /// `--model` was given on the command line: it wins over the conversation's saved model.
    pub model_explicit: bool,
    pub goal: String,
    /// `--resume`: continue the mission's paused run instead of planning `goal`.
    pub resume: bool,
    pub max_repairs: u32,
}

/// One stderr line per autopilot state change, derived from successive projections.
#[derive(Default)]
struct Progress {
    planned: bool,
    approved: BTreeSet<u64>,
    running: BTreeSet<u64>,
    started: BTreeSet<u64>,
    settled: BTreeSet<u64>,
    repairs: std::collections::BTreeMap<u64, u32>,
    state: Option<RunState>,
    /// Stream events matching the lines of the last `lines` call: (type, fields).
    events: Vec<(&'static str, serde_json::Value)>,
}
impl Progress {
    fn lines(&mut self, status: &Status, outcomes: &[TaskOutcome]) -> Vec<String> {
        let mut lines = Vec::new();
        if !self.planned && !outcomes.is_empty() {
            self.planned = true;
            lines.push("plan drafted".into());
        }
        for outcome in outcomes {
            let id = outcome.id;
            if outcome.status == TaskStatus::Approved && self.approved.insert(id) {
                lines.push(format!("#{id} approved"));
            }
            if outcome.status == TaskStatus::Running && self.running.insert(id) {
                lines.push(format!("#{id} running"));
            }
            // The poll can miss a short `Running` phase; a task that repaired or settled
            // did start, so its event is never skipped (stderr keeps its own lines).
            if (outcome.status == TaskStatus::Running
                || outcome.repairs > 0
                || outcome.result.is_some())
                && self.started.insert(id)
            {
                self.events
                    .push(("task_started", serde_json::json!({ "id": id })));
            }
            let before = self.repairs.insert(id, outcome.repairs).unwrap_or(0);
            if outcome.repairs > before {
                let cause = outcome.repair_cause.map_or("check failed", |c| c.label());
                lines.push(format!("#{id} {cause} → repair #{}", outcome.head));
                self.events.push((
                    "repair",
                    serde_json::json!({ "id": id, "cause": cause, "head": outcome.head }),
                ));
            }
            match &outcome.result {
                Some(Ok(())) if self.settled.insert(id) => {
                    lines.push(format!("#{id} accepted"));
                    self.events
                        .push(("check", serde_json::json!({ "id": id, "passed": true })));
                }
                Some(Err(reason)) if self.settled.insert(id) => {
                    let reason = failed_reason(outcome, reason);
                    lines.push(format!("#{id} failed: {reason}"));
                    self.events.push((
                        "check",
                        serde_json::json!({ "id": id, "passed": false, "reason": reason }),
                    ));
                }
                _ => {}
            }
        }
        if self.state != Some(status.state) {
            self.state = Some(status.state);
            match status.state {
                RunState::Finishing => lines.push("integrating".into()),
                // A failed run's line carries the reason, known only at the end.
                RunState::Failed => {}
                state if state.finished() => lines.push(finished_line(state, status, "")),
                _ => {}
            }
        }
        lines
    }
}

/// A stuck task's reason with its last failure line appended (the TUI's summary text).
fn failed_reason(outcome: &TaskOutcome, reason: &str) -> String {
    match &outcome.last_failure {
        Some(failure) => format!("{reason}; last failure: {failure}"),
        None => reason.to_owned(),
    }
}

/// The terminal stderr line of a finished run; a failure names its reason.
fn finished_line(state: RunState, status: &Status, reason: &str) -> String {
    let branch = status
        .branch
        .as_ref()
        .map(|branch| format!(" {branch}"))
        .unwrap_or_default();
    let why = if reason.is_empty() {
        String::new()
    } else {
        format!(": {}", bounded_reason(reason))
    };
    format!(
        "{} {}/{}{branch}{why}",
        state.label(),
        status.done,
        status.total
    )
}

/// The command-history 80% warning, said once. Measuring a history is not free, so it
/// is looked at again only when a conversation gained or lost a command.
#[derive(Default)]
struct HistoryWatch {
    warned: bool,
    lengths: Vec<usize>,
    /// How many times the histories were measured (for tests).
    measured: usize,
}
impl HistoryWatch {
    fn poll(&mut self, sessions: &[alfredo_tui::model::Session]) -> Option<String> {
        if self.warned {
            return None;
        }
        let lengths: Vec<usize> = sessions.iter().map(|s| s.commands().len()).collect();
        if lengths == self.lengths {
            return None;
        }
        self.lengths = lengths;
        self.measured += 1;
        let warning = sessions
            .iter()
            .find_map(alfredo_tui::model::Session::command_history_warning)?;
        self.warned = true;
        Some(warning)
    }
}

/// Status for a run whose saved state is unreadable.
fn stopped_status(goal: &str) -> Status {
    Status {
        state: RunState::Failed,
        goal: goal.to_owned(),
        done: 0,
        total: 0,
        failed: 0,
        repairs: 0,
        max_repairs: 0,
        elapsed: Duration::ZERO,
        branch: None,
    }
}

/// The one JSON result object.
fn result_json(
    model: &str,
    status: &Status,
    outcomes: &[TaskOutcome],
    reason: &str,
    run_error: bool,
) -> serde_json::Value {
    let mut failed: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| match &outcome.result {
            Some(Err(reason)) => {
                let mut entry = serde_json::json!({
                    "id": outcome.id, "title": outcome.title,
                    "reason": failed_reason(outcome, reason)
                });
                if let Some(failure) = &outcome.last_failure {
                    entry["last_failure"] = failure.clone().into();
                }
                Some(entry)
            }
            _ => None,
        })
        .collect();
    // A paused run (autopilot halted itself) names its cause the same way. A plain
    // pause's generic notice is no cause, so nothing is invented for it.
    let paused_with_reason = status.state == RunState::Paused
        && !reason.is_empty()
        && reason != alfredo_tui::autopilot::PAUSED_NOTICE;
    if paused_with_reason || (status.state == RunState::Failed && (failed.is_empty() || run_error))
    {
        failed.push(serde_json::json!({
            "id": null, "title": "autopilot", "reason": reason
        }));
    }
    serde_json::json!({
        "model": model,
        "state": status.state.label(),
        "goal": status.goal,
        "branch": status.branch,
        "accepted": status.done,
        "total": status.total,
        "repairs": status.repairs,
        "failed": failed,
    })
}

/// The model this run uses: an explicit `--model` as given (the conversation's saved
/// model is left alone, never written), otherwise the restored conversation's model.
fn run_model(work: &Workstation, flag: String, explicit: bool) -> String {
    if explicit {
        flag
    } else {
        work.app.sessions[work.app.selected].model.clone()
    }
}

fn preflight(runtime: &Runtime, provider: &Ollama, model: &str) -> Result<(), String> {
    runtime
        .block_on(alfredo_tui::doctor::model_server_check(provider, model))
        .map_err(|failure| alfredo_tui::doctor::model_check_message(model, &failure))
}

/// Why `--go` will not plan over a paused run, and the ways forward.
fn paused_refusal(run: &Status) -> String {
    format!(
        "--go refused: this mission has a paused run: “{}” ({}/{} accepted). Nothing was planned. \
         Continue it with --resume --headless. To start a different goal instead, pass \
         --conversation NAME (a new name has no paused run; the paused one stays resumable under \
         its own conversation) or use /go GOAL in the terminal, which replaces it",
        stderr_line(&run.goal),
        run.done,
        run.total
    )
}

/// Run the goal to its end. `Err` is a setup error; otherwise the exit code.
pub fn run(options: Options) -> Result<i32, String> {
    let Options {
        runtime,
        provider,
        state_dir,
        start,
        conversation,
        model,
        model_explicit,
        mut goal,
        resume,
        max_repairs,
    } = options;
    if !resume && alfredo_tui::wayfinder::entry_mode(&goal).is_some() {
        return Err("Wayfinder goals need the interactive terminal; state a coding goal".into());
    }
    let choice = runtime
        .block_on(alfredo_tui::selection::automatic(&start, &state_dir))?
        .ok_or_else(|| alfredo_tui::selection::not_a_repository_notice(&start))?;
    let request = alfredo_tui::selection_command::Request::new(
        alfredo_tui::selection_command::Origin::Startup,
        choice,
        conversation,
    )?;
    let checker = provider.clone();
    // Setup errors (Ollama down, model missing) are exit 2, not a failed run. An explicit
    // model is checked before launch creates and saves the conversation with it.
    if model_explicit {
        preflight(runtime, &checker, &model)?;
    }
    let mut work = Workstation::launch(runtime, &state_dir, request, &model, provider)?;
    work.tasks.refresh(runtime);
    // A paused run is never silently forked (issue #280): `--go` refuses, `--resume` continues.
    let paused = work.autopilot.paused_run(&work.tasks);
    match (&paused, resume) {
        (Some(run), false) => return Err(paused_refusal(run)),
        (Some(run), true) if alfredo_tui::wayfinder::entry_mode(&run.goal).is_some() => {
            return Err("Wayfinder goals need the interactive terminal; /resume the paused run there".into())
        }
        (None, true) => {
            return Err("--resume: nothing to resume; this mission has no paused run (--go GOAL --headless starts one)".into())
        }
        _ => {}
    }
    let model = run_model(&work, model, model_explicit);
    if !model_explicit {
        preflight(runtime, &checker, &model)?;
    }
    say(format!("model {model}"));
    if let Some(run) = paused {
        goal = run.goal;
        work.autopilot
            .resume(&work.tasks)
            .map_err(|error| format!("--resume refused: {error}"))?;
        say(format!(
            "resuming “{}” ({}/{} accepted)",
            stderr_line(&goal),
            run.done,
            run.total
        ));
    } else {
        // The start notice carries the start warning (e.g. a timed-out uncommitted-changes
        // check, #470), which headless shows on stderr like the other start lines.
        let notice = work
            .autopilot
            .start(&goal, &model, max_repairs, &work.tasks)
            .map_err(|error| format!("--go refused: {error}"))?;
        say(&notice);
    }
    work.tasks.set_visible(true);
    catch_signals();

    let mut progress = Progress::default();
    let mut pending: Option<(usize, String)> = None;
    let (mut last_refresh, mut last_save) = (Instant::now(), Instant::now());
    let mut stopping: Option<Instant> = None;
    let mut reason = String::new();
    let mut history = HistoryWatch::default();
    // A runtime failure after launch still ends with the JSON result.
    let mut fatal: Option<String> = None;
    loop {
        if stopping.is_none() && INTERRUPTED.load(Ordering::SeqCst) > 0 {
            say("interrupted: stopping workers; /resume continues this run");
            work.autopilot.stop(runtime, &mut work.tasks);
            stopping = Some(Instant::now());
        }
        if let Err(error) = step(
            runtime,
            &mut work,
            &mut pending,
            stopping.is_some() || fatal.is_some(),
            &mut last_refresh,
            &mut last_save,
        ) {
            say(format!("Error: {error}"));
            work.autopilot.stop(runtime, &mut work.tasks);
            fatal = Some(error);
            stopping.get_or_insert_with(Instant::now);
        }
        let mut status = work.autopilot.status(&work.tasks).unwrap_or_else(|| {
            fatal.get_or_insert_with(|| "Autopilot state vanished".into());
            stopped_status(&goal)
        });
        let outcomes = work.autopilot.outcomes(&work.tasks);
        for line in progress.lines(&status, &outcomes) {
            say(&line);
        }
        for (kind, fields) in std::mem::take(&mut progress.events) {
            emit(kind, fields);
        }
        if let Some(warning) = history.poll(&work.app.sessions) {
            say(&warning);
        }
        let drained = work.tasks.workers.is_empty() && !work.tasks.writing;
        let end = if let Some(since) = stopping {
            let timed_out = !drained && since.elapsed() > Duration::from_secs(30);
            let again = INTERRUPTED.load(Ordering::SeqCst) > 1;
            if !drained && (timed_out || again) {
                say(if again {
                    "second signal: not waiting for workers to finish"
                } else {
                    "workers did not finish within 30 s; reporting anyway"
                });
            }
            drained || timed_out || again
        } else if status.state.finished() {
            drained
        } else if status.state == RunState::Paused {
            // Autopilot halted itself (a refusal, contention): the notice says why.
            reason = work.autopilot.notice().to_owned();
            say(&reason);
            work.autopilot.stop(runtime, &mut work.tasks);
            stopping = Some(Instant::now());
            false
        } else {
            false
        };
        if end {
            if let Some(error) = &fatal {
                status.state = RunState::Failed;
                reason = error.clone();
            } else if status.state.finished() && status.state == RunState::Failed {
                // The report's "Reason" row names why the whole run failed.
                reason = work
                    .autopilot
                    .report()
                    .and_then(|report| report.lines().find_map(|line| line.strip_prefix("Reason")))
                    .map_or(reason, |text| text.trim().to_owned());
            }
            if status.state == RunState::Failed {
                say(finished_line(status.state, &status, &reason));
            }
            let mut result = result_json(&model, &status, &outcomes, &reason, fatal.is_some());
            result["usage"] = work.autopilot.usage(&work.tasks).json();
            print_result(result);
            // The JSON is out; now let a started hook finish (or be killed) and report.
            let grace = alfredo_tui::on_finish::TIMEOUT + Duration::from_secs(5);
            for failure in work.autopilot.finish_hooks(grace) {
                say(failure);
            }
            return Ok(i32::from(status.state != RunState::Done));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The non-key half of the TUI loop: poll task control, let autopilot choose its
/// next command, save it, and release it through the saved-intent barrier.
fn step(
    runtime: &Runtime,
    work: &mut Workstation,
    pending: &mut Option<(usize, String)>,
    stopping: bool,
    last_refresh: &mut Instant,
    last_save: &mut Instant,
) -> Result<(), String> {
    work.tasks.poll();
    for session in &mut work.app.sessions {
        session.protect_plan_origin(work.tasks.planner.pending_origin());
    }
    for event in work.tasks.take_control_events() {
        let intent = Intent::Control {
            request: event.request,
        };
        let id = alfredo_tui::console_command::ConsoleCommand::identity(&intent);
        for session in &mut work.app.sessions {
            if session
                .commands()
                .iter()
                .any(|command| command.id == id && command.intent == intent)
            {
                session.set_command_state(
                    &id,
                    CommandState::Control {
                        outcome: event.outcome.clone(),
                    },
                );
            }
        }
    }
    for event in work.tasks.planner.take_command_events() {
        for session in &mut work.app.sessions {
            let ids: Vec<_> = session
                .commands()
                .iter()
                .filter(|command| command.intent.planner_request() == Some(&event.request))
                .map(|command| command.id.clone())
                .collect();
            for id in ids {
                session.set_command_state(
                    &id,
                    CommandState::Planner {
                        outcome: event.outcome.clone(),
                    },
                );
            }
        }
    }
    for session in &mut work.app.sessions {
        let uncertain: Vec<_> = session
            .commands()
            .iter()
            .filter(|command| {
                matches!(command.state, CommandState::Submitted)
                    && !work.tasks.intent_pending(&command.intent)
                    && command
                        .intent
                        .acknowledgment(
                            work.tasks.snapshot.as_ref(),
                            work.tasks.canonical_scope.as_ref(),
                        )
                        .is_none()
            })
            .map(|command| {
                (
                    command.id.clone(),
                    work.tasks.unmatched_state(&command.intent),
                )
            })
            .collect();
        for (id, state) in uncertain {
            session.set_command_state(&id, state);
        }
    }
    if !stopping && pending.is_none() {
        crate::command_staging::submit_autopilot(runtime, work, pending);
    }
    if !stopping && pending.is_none() {
        stage_dispatch(work, pending);
    }
    if pending.is_some() || last_save.elapsed() >= Duration::from_secs(1) {
        work.autosave
            .checkpoint(
                runtime,
                &work.app,
                work.tasks.view_preferences(),
                work.tasks.planner.checkpoint(),
            )
            .map_err(|error| format!("Conversation save failed: {error}"))?;
        *last_save = Instant::now();
    }
    if !stopping {
        if let Some((origin, command)) =
            crate::command_staging::saved_pending_command(&work.app, &work.autosave, pending)
        {
            *pending = None;
            let state = match work.tasks.dispatch_prepared(runtime, &command.intent) {
                Ok(()) => CommandState::Submitted,
                Err(error) => {
                    if let Intent::DispatchRun { request } = &command.intent {
                        work.tasks.refuse_dispatch(request, &error);
                    }
                    CommandState::Refused {
                        reason: bounded_reason(&error),
                    }
                }
            };
            work.app.sessions[origin].set_command_state(&command.id, state);
        }
    }
    let period = if work.tasks.dispatch.enabled {
        Duration::from_millis(250)
    } else {
        Duration::from_secs(1)
    };
    if (!work.tasks.workers.is_empty() || work.tasks.dispatch.enabled || work.tasks.visible)
        && last_refresh.elapsed() >= period
    {
        work.tasks.refresh_background(runtime);
        *last_refresh = Instant::now();
    }
    work.sync_autopilot();
    Ok(())
}

/// Automatic Architect review and dispatch of approved tasks, as in the TUI.
fn stage_dispatch(work: &mut Workstation, pending: &mut Option<(usize, String)>) {
    match work.tasks.prepare_architect() {
        Ok(Some(request)) => {
            match crate::command_staging::stage_automatic_command(
                work,
                Intent::ArchitectDraft { request },
            ) {
                Ok(staged) => *pending = Some(staged),
                Err(error) => say(format!("Automatic Architect not started: {error}")),
            }
            return;
        }
        Ok(None) => {}
        Err(error) => {
            say(format!("Automatic Architect not started: {error}"));
            return;
        }
    }
    match work.tasks.prepare_dispatch() {
        Ok(Some(request)) => {
            match crate::command_staging::stage_automatic_command(
                work,
                Intent::DispatchRun {
                    request: request.clone(),
                },
            ) {
                Ok(staged) => *pending = Some(staged),
                Err(error) => work.tasks.refuse_dispatch(&request, &error),
            }
        }
        Ok(None) => {}
        Err(error) => {
            work.tasks.disable_dispatch();
            work.tasks.notice = error;
        }
    }
}

/// How `--chat --headless` answers the Approval gate (ADR 0004 forbids a
/// skip-approval mode; this is scripted, headless-only and scoped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// No `--approve`: every mutating call is denied (and logged).
    Deny,
    /// `--approve workspace-edits`: approve `edit_file` and `write_file` cards whose
    /// path stays inside the Coding Workspace; deny everything else.
    WorkspaceEdits,
}

impl Policy {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "workspace-edits" => Ok(Policy::WorkspaceEdits),
            other => Err(format!(
                "--approve {:?} is unknown; the only policy is workspace-edits (approve edit_file/write_file inside the Coding Workspace, deny everything else)",
                other.chars().take(40).collect::<String>()
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Policy::Deny => "deny",
            Policy::WorkspaceEdits => "workspace-edits",
        }
    }

    pub fn decide(self, card: &Card) -> Decision {
        let inside = |path: &str| {
            let path = std::path::Path::new(path.trim());
            !path.as_os_str().is_empty()
                && path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
        };
        let edit = card.kind == CardKind::Edit
            && matches!(card.tool.as_str(), "edit_file" | "write_file")
            && card
                .arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .is_some_and(inside);
        if self == Policy::WorkspaceEdits && edit {
            Decision::Approve
        } else {
            Decision::Deny
        }
    }
}

/// Class of one tool result: `ok`, `denied` (Approval refused), `malformed` (unknown
/// tool or bad arguments) or `error` (a well-formed call the tool refused).
fn classify(result: &str) -> &'static str {
    let Some(reason) = result.strip_prefix("error: ") else {
        return "ok";
    };
    if result == alfredo_tui::chat_tools::approval::DENIED {
        return "denied";
    }
    let malformed = reason.starts_with("unknown tool ")
        || reason.starts_with("arguments must be")
        || reason.starts_with("missing required argument")
        || (reason.starts_with("argument ")
            && (reason.contains(" must be ") || reason.ends_with(" is required")))
        || reason.contains("must be a positive integer");
    if malformed {
        "malformed"
    } else {
        "error"
    }
}

/// One tool call of the turn: name, `path` argument, result class, and for a result
/// that is not `ok` the tool's error text (bounded, no control characters).
struct Call {
    tool: String,
    path: Option<String>,
    result: &'static str,
    detail: Option<String>,
}

/// Longest `detail` of a call, in bytes.
const DETAIL_MAX: usize = 200;

/// The tool's error text without its `error: ` prefix, control characters removed,
/// cut to [`DETAIL_MAX`] bytes on a character boundary.
fn detail_of(result: &str) -> String {
    let text = result.strip_prefix("error: ").unwrap_or(result);
    let mut detail = String::new();
    for c in text.chars().filter(|c| !c.is_control()) {
        if detail.len() + c.len_utf8() > DETAIL_MAX {
            break;
        }
        detail.push(c);
    }
    detail
}

/// What one headless chat turn produced, from its events.
#[derive(Default)]
struct Turn {
    reply: String,
    steps: usize,
    calls: Vec<Call>,
    done: bool,
    failure: Option<String>,
    /// Set when a cap ended the turn (issue #278): state `stopped`, exit 1.
    stopped: Option<Stop>,
    /// Set when the model has no tool support (issue #281): the reply stays the model's text.
    no_tools: bool,
    /// Round cap the turn ran under (`--max-turns`); None is the default.
    cap: Option<usize>,
    /// Server counts summed over the turn's rounds (issue #283).
    usage: alfredo_tui::metrics::Usage,
}

impl Turn {
    fn update(&mut self, update: Update) {
        match update {
            // After a stop the text is only the TUI's "ask again" notice.
            Update::Token(text) if self.stopped.is_none() => self.reply.push_str(&text),
            Update::Stopped(stop) => self.stopped = Some(stop),
            Update::Notice(text) => {
                self.no_tools = true;
                say(text.chars().filter(|c| !c.is_control()).collect::<String>());
            }
            Update::ToolStep(step) => {
                self.steps += 1;
                for (call, result) in step.calls.iter().zip(&step.results) {
                    self.record(call, result);
                }
            }
            Update::ToolResult {
                call,
                result,
                new_round,
            } => {
                self.steps += usize::from(new_round);
                self.record(&call, &result);
            }
            // An empty reply is a failed turn, as in the TUI (issue #397). A capped turn's
            // text is only the "ask again" notice, so it is judged by its stop.
            Update::Done if self.stopped.is_none() && self.reply.trim().is_empty() => {
                self.failure = Some(alfredo_tui::model::NO_TEXT.into());
            }
            Update::Done => self.done = true,
            Update::Failed(reason) => self.failure = Some(reason),
            Update::Metrics(metrics) => {
                if let Some(usage) = alfredo_tui::metrics::Usage::from_metrics(&metrics) {
                    self.usage.add(&usage);
                }
            }
            _ => {}
        }
    }

    fn record(&mut self, call: &alfredo_tui::model::ToolCall, text: &str) {
        let result = classify(text);
        let path = call
            .arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let detail = (result != "ok").then(|| detail_of(text));
        let target: String = path
            .iter()
            .flat_map(|p| p.chars().filter(|c| !c.is_control()).take(DETAIL_MAX))
            .collect();
        say(format!(
            "tool {}{}{target} ({result}{})",
            call.name,
            if target.is_empty() { "" } else { " " },
            detail
                .as_ref()
                .map_or_else(String::new, |d| format!(": {d}")),
        ));
        let mut event = serde_json::json!({ "tool": call.name, "path": path, "result": result });
        if let Some(detail) = &detail {
            event["detail"] = detail.clone().into();
        }
        emit("tool", event);
        self.calls.push(Call {
            tool: call.name.clone(),
            path,
            result,
            detail,
        });
    }

    fn json(&self, model: &str, policy: Policy, records: &[Record]) -> serde_json::Value {
        let error = match (&self.failure, self.done, self.stopped) {
            (Some(reason), _, _) => Some(reason.clone()),
            (None, _, Some(Stop::StepLimit)) => Some(format!(
                "{STEP_CAP_NOTICE} after {} rounds; the turn was cut off",
                self.cap.unwrap_or(MAX_STEPS)
            )),
            (None, _, Some(Stop::OutputLimit)) => {
                Some("the conversation is near its size limit; the turn was cut off".to_string())
            }
            (None, true, None) => None,
            (None, false, None) => Some("the turn ended without a reply".to_string()),
        };
        let approvals: Vec<_> = records
            .iter()
            .map(|record| {
                let (outcome, reason) = match &record.outcome {
                    Outcome::Applied => ("applied", None),
                    Outcome::Failed(reason) => ("failed", Some(reason.clone())),
                    Outcome::Denied => ("denied", None),
                    Outcome::Cancelled => ("cancelled", None),
                };
                serde_json::json!({
                    "tool": record.tool, "target": record.target,
                    "outcome": outcome, "reason": reason,
                })
            })
            .collect();
        serde_json::json!({
            "model": model,
            "state": match (&error, self.stopped) {
                (None, _) => "done",
                (_, Some(_)) => "stopped",
                _ => "failed",
            },
            "stopped": self.stopped.map(Stop::label),
            "policy": policy.label(),
            "reply": self.reply.trim(),
            "tools": !self.no_tools,
            "steps": self.steps,
            "calls": self.calls.iter().map(|call| serde_json::json!({
                "tool": call.tool, "path": call.path,
                "error": call.result != "ok", "result": call.result,
                "detail": call.detail,
            })).collect::<Vec<_>>(),
            "approvals": approvals,
            "error": error,
            "usage": self.usage.json(),
        })
    }
}

pub struct ChatOptions<'a> {
    pub runtime: &'a Runtime,
    pub provider: Ollama,
    pub state_dir: PathBuf,
    pub start: PathBuf,
    pub conversation: String,
    pub model: String,
    /// `--model` was given on the command line: it wins over the conversation's saved model.
    pub model_explicit: bool,
    pub prompt: String,
    pub policy: Policy,
}

/// `--chat PROMPT --headless`: one Chat Agent turn (tools, Approval gate answered by
/// `policy`, outcomes in the chat-approval log) on the repository's default mission.
/// One JSON object on stdout; exit 0 when the turn finished, 1 failed or interrupted.
/// The turn is not saved to the conversation.
pub fn run_chat(options: ChatOptions) -> Result<i32, String> {
    let ChatOptions {
        runtime,
        provider,
        state_dir,
        start,
        conversation,
        model,
        model_explicit,
        prompt,
        policy,
    } = options;
    let chat = provider.clone();
    let mut checked = false;
    // Read-only: the turn is not saved, so it neither owns the conversation nor
    // takes the selection journal, and runs beside an open TUI or another chat.
    // Another run creating the default mission at the same moment can make either
    // step report it missing or existing: look again, now that it is settled.
    let mut attempt = 0;
    let view = loop {
        attempt += 1;
        let opened = (|| {
            let choice = runtime
                .block_on(alfredo_tui::selection::automatic(&start, &state_dir))?
                .ok_or_else(|| alfredo_tui::selection::not_a_repository_notice(&start))?;
            if model_explicit && !checked {
                preflight(runtime, &chat, &model)?;
                checked = true;
            }
            alfredo_tui::workstation::ChatView::open(
                &state_dir,
                &choice.target(),
                choice.mission.name(),
                choice.mission.start_new(),
                &conversation,
            )
        })();
        match opened {
            Err(error)
                if attempt < 20
                    && (error == alfredo_tui::tasks::MISSION_MISSING
                        || error == alfredo_tui::tasks::MISSION_EXISTS) =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            other => break other?,
        }
    };
    let model = if model_explicit {
        model
    } else {
        view.saved_model.unwrap_or(model)
    };
    if !model_explicit {
        preflight(runtime, &chat, &model)?;
    }
    let work = view.tasks;
    let workspace = work.workspace().to_path_buf();
    let log = Log::for_store(work.store()).ok();
    let before = log.as_ref().map_or(0, |log| log.records().len());
    let (context, notes) = work.chat_context_with_notes();
    if let Some((_, line)) = notes {
        say(line);
    }
    // Same `@path` rules and bounds as interactive chat, on the workspace root the
    // read tools are confined to (issue #282).
    let attachment = alfredo_tui::chat_mentions::attach(&workspace, &prompt);
    for notice in &attachment.notices {
        say(notice);
    }
    let content = format!("{prompt}{}", attachment.block);
    let messages = vec![
        context,
        alfredo_tui::model::Message {
            role: "user".into(),
            content,
        },
    ];
    say(format!(
        "chat · model {model} · approvals: {}",
        policy.label()
    ));
    catch_signals();
    let (requests, mut asked) = tokio::sync::mpsc::channel(4);
    let gate = Gate::new(requests, log.clone());
    let (sender, mut events) = tokio::sync::mpsc::channel(256);
    let mut turn = Turn::default();
    // Decisions sent; after a signal every later card is denied, and the turn is only
    // dropped once each answered card is logged (an approved edit runs and logs itself).
    let answered = std::sync::atomic::AtomicUsize::new(0);
    let closing = std::sync::atomic::AtomicBool::new(false);
    let logged = || {
        log.as_ref()
            .map_or(usize::MAX, |log| log.records().len().saturating_sub(before))
    };
    runtime.block_on(async {
        let run = alfredo_tui::chat_tools::run_turn(
            &chat,
            &workspace,
            0,
            1,
            model.clone(),
            messages,
            sender,
            Some(gate),
        );
        let answer = async {
            while let Some(request) = asked.recv().await {
                let decision = if closing.load(Ordering::SeqCst) {
                    Decision::Deny
                } else {
                    policy.decide(&request.card)
                };
                answered.fetch_add(1, Ordering::SeqCst);
                say(format!(
                    "approval {}: {}",
                    request.card.summary,
                    if decision == Decision::Approve {
                        "approved"
                    } else {
                        "denied"
                    }
                ));
                let _ = request.reply.send(decision);
            }
        };
        let collect = async {
            while let Some(event) = events.recv().await {
                turn.update(event.update);
            }
        };
        let stop = async {
            while INTERRUPTED.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            closing.store(true, Ordering::SeqCst);
            let since = Instant::now();
            while logged() < answered.load(Ordering::SeqCst)
                && since.elapsed() < Duration::from_secs(10)
                && INTERRUPTED.load(Ordering::SeqCst) < 2
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::select! {
            _ = async { tokio::join!(run, answer, collect) } => {}
            () = stop => {}
        }
    });
    if INTERRUPTED.load(Ordering::SeqCst) > 0 {
        say("interrupted: turn cancelled");
        turn.failure = Some("interrupted".into());
        turn.stopped = None;
    }
    turn.cap = chat.max_turns();
    let records = log.map_or_else(Vec::new, |log| log.records());
    let mut value = turn.json(&model, policy, records.get(before..).unwrap_or(&[]));
    if !attachment.files.is_empty() {
        value["attached"] = attachment.files.into();
    }
    let state_done = value["state"] == "done";
    print_result(value);
    Ok(i32::from(!state_done))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_warning_fires_once_and_is_not_remeasured_while_counts_hold() {
        use alfredo_tui::tasks::Request;
        let mut app = alfredo_tui::model::App::new("fixture".into());
        let mut watch = HistoryWatch::default();
        let (mut said, mut changes) = (Vec::new(), 0);
        for number in 0..500 {
            let polls = if number == 0 { 1 } else { 3 };
            for _ in 0..polls {
                said.extend(watch.poll(&app.sessions));
            }
            if watch.warned {
                break;
            }
            changes += 1;
            assert_eq!(watch.measured, changes, "one measurement per change");
            app.sessions[0]
                .submit_command(
                    "/task Live".into(),
                    Intent::Task {
                        request: Request {
                            correlation: format!("watch-{number}"),
                            expected_revision: 0,
                            action: alfredo_tui::tasks::Action::Propose {
                                title: "Proposed command task".into(),
                                model: "fixture".into(),
                                dependencies: vec![],
                            },
                        },
                    },
                )
                .unwrap();
        }
        assert!(watch.warned, "the history crossed 80%");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("80%") && said[0].contains("/clear"),
            "{said:?}"
        );
        let measured = watch.measured;
        // More commands, and repeated polls: nothing more is said or measured.
        let mut again = app.sessions[0].commands()[0].intent.clone();
        if let Intent::Task { request } = &mut again {
            request.correlation = "after".into();
        }
        app.sessions[0]
            .submit_command("/task Live".into(), again)
            .unwrap();
        assert_eq!(watch.poll(&app.sessions), None);
        assert_eq!(watch.measured, measured);
    }

    fn status(state: RunState, done: usize) -> Status {
        Status {
            state,
            goal: "g".into(),
            done,
            total: 2,
            failed: 0,
            repairs: 0,
            max_repairs: 3,
            elapsed: Duration::ZERO,
            branch: Some("alfredo/go-ab12".into()),
        }
    }
    fn outcome(
        id: u64,
        status: TaskStatus,
        repairs: u32,
        result: Option<Result<(), String>>,
    ) -> TaskOutcome {
        TaskOutcome {
            id,
            title: format!("t{id}"),
            head: id + 2,
            status,
            repairs,
            result,
            repair_cause: None,
            last_failure: None,
        }
    }

    #[test]
    fn progress_prints_each_change_once() {
        let mut progress = Progress::default();
        assert!(progress
            .lines(&status(RunState::Planning, 0), &[])
            .is_empty());
        let open = [outcome(1, TaskStatus::Approved, 0, None)];
        assert_eq!(
            progress.lines(&status(RunState::Running, 0), &open),
            ["plan drafted", "#1 approved"]
        );
        assert!(progress
            .lines(&status(RunState::Running, 0), &open)
            .is_empty());
        let repaired = [outcome(1, TaskStatus::Failed, 1, None)];
        assert_eq!(
            progress.lines(&status(RunState::Running, 0), &repaired),
            ["#1 check failed → repair #3"]
        );
        let done = [outcome(1, TaskStatus::Accepted, 1, Some(Ok(())))];
        assert_eq!(
            progress.lines(&status(RunState::Done, 2), &done),
            ["#1 accepted", "done 2/2 alfredo/go-ab12"]
        );
    }

    #[test]
    fn progress_events_start_a_task_once_then_repair_and_check() {
        let mut progress = Progress::default();
        let events = |progress: &mut Progress| std::mem::take(&mut progress.events);
        progress.lines(
            &status(RunState::Running, 0),
            &[outcome(1, TaskStatus::Approved, 0, None)],
        );
        assert!(events(&mut progress).is_empty());
        let running = [outcome(1, TaskStatus::Running, 0, None)];
        progress.lines(&status(RunState::Running, 0), &running);
        progress.lines(&status(RunState::Running, 0), &running);
        assert_eq!(
            events(&mut progress),
            [("task_started", serde_json::json!({"id": 1}))]
        );
        progress.lines(
            &status(RunState::Running, 0),
            &[outcome(1, TaskStatus::Failed, 1, None)],
        );
        assert_eq!(
            events(&mut progress),
            [(
                "repair",
                serde_json::json!({"id": 1, "cause": "check failed", "head": 3})
            )]
        );
        // A task whose Running phase was never polled still gets its start before its check.
        progress.lines(
            &status(RunState::Running, 0),
            &[
                outcome(1, TaskStatus::Accepted, 1, Some(Ok(()))),
                outcome(2, TaskStatus::Failed, 0, Some(Err("bad".into()))),
            ],
        );
        assert_eq!(
            events(&mut progress),
            [
                ("check", serde_json::json!({"id": 1, "passed": true})),
                ("task_started", serde_json::json!({"id": 2})),
                (
                    "check",
                    serde_json::json!({"id": 2, "passed": false, "reason": "bad"})
                ),
            ]
        );
    }

    #[test]
    fn progress_starts_a_task_first_seen_repairing_before_its_repair_event() {
        let mut progress = Progress::default();
        progress.lines(
            &status(RunState::Running, 0),
            &[outcome(1, TaskStatus::Failed, 1, None)],
        );
        let names: Vec<_> = progress.events.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["task_started", "repair"]);
        assert_eq!(progress.events[0].1, serde_json::json!({"id": 1}));
    }

    #[test]
    fn progress_reports_a_settled_task_once_across_repeated_polls() {
        let mut progress = Progress::default();
        let outcomes = [
            outcome(1, TaskStatus::Accepted, 0, Some(Ok(()))),
            outcome(2, TaskStatus::Failed, 0, Some(Err("bad".into()))),
        ];
        let mut lines = Vec::new();
        for _ in 0..2 {
            lines.extend(progress.lines(&status(RunState::Running, 0), &outcomes));
        }
        assert_eq!(lines.iter().filter(|l| *l == "#1 accepted").count(), 1);
        assert_eq!(
            lines.iter().filter(|l| l.starts_with("#2 failed")).count(),
            1
        );
        let checks: Vec<_> = progress
            .events
            .iter()
            .filter(|(name, _)| *name == "check")
            .map(|(_, fields)| fields.clone())
            .collect();
        assert_eq!(
            checks,
            [
                serde_json::json!({"id": 1, "passed": true}),
                serde_json::json!({"id": 2, "passed": false, "reason": "bad"}),
            ]
        );
    }

    #[test]
    fn repair_line_names_its_trigger_and_failed_task_carries_the_last_failure() {
        use alfredo_tui::autopilot::RepairCause;
        let mut progress = Progress::default();
        let mut repaired = outcome(1, TaskStatus::Failed, 1, None);
        repaired.repair_cause = Some(RepairCause::NoFilesChanged);
        assert_eq!(
            progress.lines(&status(RunState::Running, 0), &[repaired]),
            ["plan drafted", "#1 no files changed → repair #3"]
        );
        let mut stuck = outcome(
            2,
            TaskStatus::Failed,
            3,
            Some(Err("failed; repair budget exhausted (3/3)".into())),
        );
        stuck.repair_cause = Some(RepairCause::CheckFailed);
        stuck.last_failure = Some("Check failed (exit 1): AssertionError".into());
        let lines = progress.lines(&status(RunState::Running, 0), &[stuck.clone()]);
        assert_eq!(
            lines,
            [
                "#2 check failed → repair #4",
                "#2 failed: failed; repair budget exhausted (3/3); last failure: Check failed (exit 1): AssertionError"
            ]
        );
        let json = result_json("m", &status(RunState::Failed, 0), &[stuck], "", false);
        let entry = &json["failed"][0];
        assert_eq!(
            entry["last_failure"],
            "Check failed (exit 1): AssertionError"
        );
        assert!(entry["reason"]
            .as_str()
            .unwrap()
            .starts_with("failed; repair budget exhausted (3/3); last failure: Check failed"));
        assert_eq!(entry["id"], 2);
        assert_eq!(entry["title"], "t2");
    }

    #[test]
    fn failed_line_carries_the_bounded_reason() {
        let failed = status(RunState::Failed, 0);
        assert_eq!(
            finished_line(RunState::Failed, &failed, "Cannot reach\nOllama"),
            "failed 0/2 alfredo/go-ab12: Cannot reachOllama"
        );
        assert_eq!(
            finished_line(RunState::Failed, &failed, ""),
            "failed 0/2 alfredo/go-ab12"
        );
        let mut progress = Progress::default();
        assert!(progress.lines(&failed, &[]).is_empty());
    }

    fn card(tool: &str, kind: CardKind, path: &str) -> Card {
        Card {
            tool: tool.into(),
            arguments: serde_json::json!({ "path": path }),
            kind,
            summary: format!("edit {path}"),
            preview: String::new(),
            fingerprint: String::new(),
        }
    }

    #[test]
    fn workspace_edits_approves_only_edit_tools_inside_the_workspace() {
        let policy = Policy::parse("workspace-edits").unwrap();
        for tool in ["edit_file", "write_file"] {
            assert_eq!(
                policy.decide(&card(tool, CardKind::Edit, "src/a.py")),
                Decision::Approve
            );
            for path in ["../a.py", "/etc/passwd", "", "a/../../b"] {
                assert_eq!(
                    policy.decide(&card(tool, CardKind::Edit, path)),
                    Decision::Deny,
                    "{path}"
                );
            }
        }
        assert_eq!(
            policy.decide(&card("run_command", CardKind::Edit, "a.py")),
            Decision::Deny
        );
        assert_eq!(
            Policy::Deny.decide(&card("edit_file", CardKind::Edit, "a.py")),
            Decision::Deny
        );
        assert!(Policy::parse("all")
            .unwrap_err()
            .contains("workspace-edits"));
    }

    #[test]
    fn results_are_classified_ok_denied_malformed_or_tool_error() {
        use alfredo_tui::chat_tools::approval::DENIED;
        assert_eq!(classify("fn x"), "ok");
        assert_eq!(classify(DENIED), "denied");
        for malformed in [
            "error: unknown tool \"x\"; available: read_file",
            "error: arguments must be a JSON object",
            "error: argument path must be a string",
            "error: argument path is required",
            "error: missing required argument pattern",
            "error: argument max_count must be a positive integer",
            "error: start_line must be a positive integer",
        ] {
            assert_eq!(classify(malformed), "malformed", "{malformed}");
        }
        for tool in [
            "error: old_text was not found in units.py",
            "error: no such file or directory: x",
            "error: .env is excluded (Git metadata, secrets or generated files)",
        ] {
            assert_eq!(classify(tool), "error", "{tool}");
        }
    }

    #[test]
    fn calls_keep_their_path_argument() {
        let mut turn = Turn::default();
        turn.update(Update::ToolStep(alfredo_tui::model::ToolStep {
            calls: vec![alfredo_tui::model::ToolCall {
                name: "read_file".into(),
                arguments: serde_json::json!({"path": "units.py"}),
            }],
            results: vec!["text".into()],
        }));
        turn.update(Update::Done);
        let value = turn.json("m", Policy::Deny, &[]);
        assert_eq!(value["calls"][0]["path"], "units.py");
        assert_eq!(value["calls"][0]["result"], "ok");
    }

    #[test]
    fn stderr_lines_carry_no_control_characters_or_line_breaks() {
        assert_eq!(
            stderr_line("approval new file esc\u{1b}]0;PWNED\u{7}name.txt: approved"),
            "approval new file esc]0;PWNEDname.txt: approved"
        );
        assert_eq!(stderr_line("a\nb\tc\u{9b}d\r"), "a b cd");
        assert_eq!(
            stderr_line("#3 failed: x\n#4 accepted"),
            "#3 failed: x #4 accepted"
        );
        assert_eq!(stderr_line("plain — text"), "plain — text");
    }

    /// Issue #361: a model-chosen path with a reversed-order override (U+202E) and a line
    /// separator (U+2028) must not reach stderr raw; the override shows as `<U+202E>` and
    /// the separator becomes a space, so no second line can be forged.
    #[test]
    fn stderr_lines_show_format_characters_and_line_separators_inline() {
        let line = stderr_line("approval write_file gpj.\u{202E}txt: approved\u{2028}#9 done");
        assert_eq!(
            line,
            "approval write_file gpj.<U+202E>txt: approved #9 done"
        );
        assert!(!line.contains(['\u{202E}', '\u{2028}', '\u{2029}', '\u{85}']));
    }

    #[test]
    fn detail_is_bounded_single_line_text_without_the_error_prefix() {
        assert_eq!(detail_of("error: no such file\x1b[0m\n"), "no such file[0m");
        let long = format!("error: {}", "é".repeat(300));
        let detail = detail_of(&long);
        assert!(detail.len() <= DETAIL_MAX && detail.len() >= DETAIL_MAX - 1);
        assert!(detail.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_stop_is_its_own_state_and_hides_the_ask_again_notice() {
        let mut turn = Turn::default();
        turn.update(Update::Token("partial".into()));
        turn.update(Update::Stopped(Stop::StepLimit));
        turn.update(Update::Token("\n\n[... ask again to continue]".into()));
        turn.update(Update::Done);
        let value = turn.json("m", Policy::Deny, &[]);
        assert_eq!(value["state"], "stopped");
        assert_eq!(value["stopped"], "step_limit");
        assert_eq!(value["reply"], "partial");
        assert!(value["error"].as_str().unwrap().contains(STEP_CAP_NOTICE));
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains(&MAX_STEPS.to_string()));
        let mut limit = Turn::default();
        limit.update(Update::Stopped(Stop::OutputLimit));
        limit.update(Update::Failed("Conversation output limit reached".into()));
        let value = limit.json("m", Policy::Deny, &[]);
        assert_eq!(value["state"], "stopped");
        assert_eq!(value["stopped"], "output_limit");
        assert_eq!(value["error"], "Conversation output limit reached");
    }

    #[test]
    fn a_notice_stays_out_of_the_reply_and_marks_tools_false() {
        let mut turn = Turn::default();
        turn.update(Update::Notice("Model m has no tool support".into()));
        turn.update(Update::Token("Hi".into()));
        turn.update(Update::Done);
        let value = turn.json("m", Policy::Deny, &[]);
        assert_eq!(value["reply"], "Hi");
        assert_eq!(value["tools"], false);
        let mut plain = Turn::default();
        plain.update(Update::Done);
        assert_eq!(plain.json("m", Policy::Deny, &[])["tools"], true);
    }

    #[test]
    fn chat_json_reports_calls_approvals_and_state() {
        let mut turn = Turn::default();
        turn.update(Update::Token("Hi".into()));
        turn.update(Update::ToolStep(alfredo_tui::model::ToolStep {
            calls: vec![alfredo_tui::model::ToolCall {
                name: "edit_file".into(),
                arguments: serde_json::json!({}),
            }],
            results: vec!["error: denied".into()],
        }));
        turn.update(Update::Done);
        let records = [Record {
            at_ms: 1,
            tool: "edit_file".into(),
            target: "edit a.py".into(),
            outcome: Outcome::Failed("changed".into()),
            escalated: false,
            rule: None,
        }];
        let value = turn.json("m", Policy::WorkspaceEdits, &records);
        assert_eq!(value["state"], "done");
        assert_eq!(value["reply"], "Hi");
        assert_eq!(value["steps"], 1);
        assert_eq!(value["calls"][0]["error"], true);
        assert_eq!(value["calls"][0]["result"], "error");
        assert_eq!(value["calls"][0]["path"], serde_json::Value::Null);
        assert_eq!(value["approvals"][0]["outcome"], "failed");
        assert_eq!(value["approvals"][0]["reason"], "changed");
        assert_eq!(value["error"], serde_json::Value::Null);
        let mut failed = Turn::default();
        failed.update(Update::Failed("down".into()));
        assert_eq!(failed.json("m", Policy::Deny, &[])["state"], "failed");
        assert_eq!(
            Turn::default().json("m", Policy::Deny, &[])["state"],
            "failed"
        );
    }

    #[test]
    fn chat_usage_sums_rounds_and_stays_null_without_counts() {
        use alfredo_tui::metrics::Metrics;
        let round = |prompt, eval, ns| {
            Update::Metrics(Metrics {
                prompt_eval_count: prompt,
                eval_count: eval,
                total_duration: ns,
                ..Metrics::default()
            })
        };
        let mut turn = Turn::default();
        turn.update(round(Some(50), Some(3), Some(1_500_000_000)));
        turn.update(round(Some(70), Some(5), None));
        assert_eq!(
            turn.json("m", Policy::Deny, &[])["usage"],
            serde_json::json!({"prompt_tokens": 120, "completion_tokens": 8, "model_seconds": 1.5})
        );
        assert_eq!(
            Turn::default().json("m", Policy::Deny, &[])["usage"],
            serde_json::json!({"prompt_tokens": null, "completion_tokens": null, "model_seconds": null})
        );
    }

    #[test]
    fn json_names_the_reason_of_a_paused_run() {
        let reason = "Autopilot paused: command not recorded: Command history capacity reached; start another conversation";
        let paused = result_json("m", &status(RunState::Paused, 0), &[], reason, false);
        assert_eq!(paused["state"], "paused");
        assert_eq!(
            paused["failed"],
            serde_json::json!([{"id": null, "title": "autopilot", "reason": reason}])
        );
        // No reason known: nothing invented.
        let unknown = result_json("m", &status(RunState::Paused, 0), &[], "", false);
        assert_eq!(unknown["failed"], serde_json::json!([]));
        // A plain pause's generic notice is not a cause.
        let plain = result_json(
            "m",
            &status(RunState::Paused, 0),
            &[],
            alfredo_tui::autopilot::PAUSED_NOTICE,
            false,
        );
        assert_eq!(plain["failed"], serde_json::json!([]));
        // A failed task stays listed once; the pause reason joins it.
        let outcomes = [outcome(
            2,
            TaskStatus::Failed,
            0,
            Some(Err("rejected by review".into())),
        )];
        let both = result_json("m", &status(RunState::Paused, 0), &outcomes, reason, false);
        assert_eq!(both["failed"].as_array().unwrap().len(), 2);
        assert_eq!(both["failed"][1]["reason"], reason);
    }

    #[test]
    fn json_lists_failures_and_names_a_whole_run_failure() {
        let outcomes = [outcome(
            2,
            TaskStatus::Failed,
            0,
            Some(Err("rejected by review".into())),
        )];
        let value = result_json("m", &status(RunState::Partial, 1), &outcomes, "", false);
        assert_eq!(value["failed"][0]["reason"], "rejected by review");
        assert_eq!(value["state"], "partial");
        let value = result_json(
            "m",
            &status(RunState::Failed, 0),
            &[],
            "planner gave up",
            false,
        );
        assert_eq!(value["failed"][0]["reason"], "planner gave up");
        // A runtime error is listed even beside failed tasks.
        let value = result_json(
            "m",
            &status(RunState::Failed, 0),
            &outcomes,
            "save failed",
            true,
        );
        assert_eq!(value["failed"][1]["reason"], "save failed");
        assert_eq!(value["failed"][1]["id"], serde_json::Value::Null);
    }
}
