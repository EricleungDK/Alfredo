//! `--go GOAL --headless`: the autopilot without a terminal. Builds the same
//! workstation the TUI uses and drives the same task-control, autopilot and
//! saved-intent barrier in its own loop; progress goes to stderr, the result is
//! one JSON object on stdout and the exit code tells the outcome (0 all accepted,
//! 1 partial/failed/interrupted; setup and usage errors are the caller's 2).
use alfredo_tui::{
    autopilot::{RunState, Status, TaskOutcome},
    command_intent::Intent,
    console_command::{bounded_reason, CommandState},
    provider::Ollama,
    tasks::TaskStatus,
    workstation::Workstation,
};
use std::{
    collections::BTreeSet,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

static INTERRUPTED: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_signal(_: libc::c_int) {
    INTERRUPTED.fetch_add(1, Ordering::SeqCst);
}

/// Progress line on stderr; a closed pipe must not abort the run.
fn say(line: impl std::fmt::Display) {
    let _ = writeln!(std::io::stderr(), "{line}");
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
    pub goal: String,
    pub max_repairs: u32,
}

/// One stderr line per autopilot state change, derived from successive projections.
#[derive(Default)]
struct Progress {
    planned: bool,
    approved: BTreeSet<u64>,
    running: BTreeSet<u64>,
    settled: BTreeSet<u64>,
    repairs: std::collections::BTreeMap<u64, u32>,
    state: Option<RunState>,
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
            let before = self.repairs.insert(id, outcome.repairs).unwrap_or(0);
            if outcome.repairs > before {
                lines.push(format!("#{id} check failed → repair #{}", outcome.head));
            }
            match &outcome.result {
                Some(Ok(())) if self.settled.insert(id) => lines.push(format!("#{id} accepted")),
                Some(Err(reason)) if self.settled.insert(id) => {
                    lines.push(format!("#{id} failed: {reason}"))
                }
                _ => {}
            }
        }
        if self.state != Some(status.state) {
            self.state = Some(status.state);
            match status.state {
                RunState::Finishing => lines.push("integrating".into()),
                state if state.finished() => lines.push(format!(
                    "{} {}/{}{}",
                    state.label(),
                    status.done,
                    status.total,
                    status
                        .branch
                        .as_ref()
                        .map(|branch| format!(" {branch}"))
                        .unwrap_or_default()
                )),
                _ => {}
            }
        }
        lines
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
    status: &Status,
    outcomes: &[TaskOutcome],
    reason: &str,
    run_error: bool,
) -> serde_json::Value {
    let mut failed: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| match &outcome.result {
            Some(Err(reason)) => Some(serde_json::json!({
                "id": outcome.id, "title": outcome.title, "reason": reason
            })),
            _ => None,
        })
        .collect();
    if status.state == RunState::Failed && (failed.is_empty() || run_error) {
        failed.push(serde_json::json!({
            "id": null, "title": "autopilot", "reason": reason
        }));
    }
    serde_json::json!({
        "state": status.state.label(),
        "goal": status.goal,
        "branch": status.branch,
        "accepted": status.done,
        "total": status.total,
        "repairs": status.repairs,
        "failed": failed,
    })
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
        goal,
        max_repairs,
    } = options;
    if alfredo_tui::wayfinder::entry_mode(&goal).is_some() {
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
    let mut work = Workstation::launch(runtime, &state_dir, request, &model, provider)?;
    work.tasks.refresh(runtime);
    let model = work.app.sessions[work.app.selected].model.clone();
    work.autopilot
        .start(&goal, &model, max_repairs, &work.tasks)
        .map_err(|error| format!("--go refused: {error}"))?;
    work.tasks.set_visible(true);
    catch_signals();

    let mut progress = Progress::default();
    let mut pending: Option<(usize, String)> = None;
    let (mut last_refresh, mut last_save) = (Instant::now(), Instant::now());
    let mut stopping: Option<Instant> = None;
    let mut reason = String::new();
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
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                result_json(&status, &outcomes, &reason, fatal.is_some())
            );
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
        crate::submit_autopilot(runtime, work, pending);
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
            crate::saved_pending_command(&work.app, &work.autosave, pending)
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
            match crate::stage_automatic_command(work, Intent::ArchitectDraft { request }) {
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
            match crate::stage_automatic_command(
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn json_lists_failures_and_names_a_whole_run_failure() {
        let outcomes = [outcome(
            2,
            TaskStatus::Failed,
            0,
            Some(Err("rejected by review".into())),
        )];
        let value = result_json(&status(RunState::Partial, 1), &outcomes, "", false);
        assert_eq!(value["failed"][0]["reason"], "rejected by review");
        assert_eq!(value["state"], "partial");
        let value = result_json(&status(RunState::Failed, 0), &[], "planner gave up", false);
        assert_eq!(value["failed"][0]["reason"], "planner gave up");
        // A runtime error is listed even beside failed tasks.
        let value = result_json(&status(RunState::Failed, 0), &outcomes, "save failed", true);
        assert_eq!(value["failed"][1]["reason"], "save failed");
        assert_eq!(value["failed"][1]["id"], serde_json::Value::Null);
    }
}
