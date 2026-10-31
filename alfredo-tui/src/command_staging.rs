use alfredo_tui::{provider::Ollama, workstation::Workstation};
use tokio::{runtime::Runtime, sync::mpsc, task::JoinHandle};

/// A finished summary request: session, attempt, what it replaces, and the note or reason.
pub(crate) type Compacted = (
    usize,
    u64,
    alfredo_tui::model::compaction::Plan,
    Result<String, String>,
);

/// Start a summary request for the oldest turns of `session` (issue #187). The job
/// reports through `results`; the session keeps its draft until then.
pub(crate) fn spawn_compaction(
    runtime: &Runtime,
    provider: &Ollama,
    results: &mpsc::Sender<Compacted>,
    job: &mut Option<JoinHandle<()>>,
    session: &mut alfredo_tui::model::Session,
    index: usize,
    send: bool,
) -> Result<(), &'static str> {
    let plan = session.start_compaction(provider.num_ctx(), send)?;
    let (provider, results) = (provider.clone(), results.clone());
    let (model, attempt) = (session.model.clone(), session.attempt);
    if let Some(old) = job.take() {
        old.abort();
    }
    *job = Some(runtime.spawn(async move {
        let result = alfredo_tui::model::compaction::summarize(&provider, model, &plan).await;
        let _ = results.send((index, attempt, plan, result)).await;
    }));
    Ok(())
}

/// Keep prepared routes in their original sessions while the single publication
/// barrier is occupied. Cancellation removes only an unsent route, never a write.
pub(crate) fn stage_ready_wayfinder(
    work: &mut Workstation,
    pending: &mut Option<(usize, String)>,
    quitting: bool,
) -> bool {
    use alfredo_tui::{
        command_intent::Intent,
        console_command::{CommandState, ConsoleCommand},
    };
    let mut changed = false;
    for (origin, attempt, user_message, request) in work.wayfinder.prepared_origins() {
        let Some(session) = work.app.sessions.get(origin) else {
            continue;
        };
        let cancelled =
            quitting || (attempt != 0 && (session.attempt != attempt || !session.status.active()));
        if pending.is_some() && !cancelled {
            continue;
        }
        let intent = Intent::Wayfinder {
            request: request.clone(),
            user_message,
        };
        let id = ConsoleCommand::identity(&intent);
        let exists = session.commands().iter().find(|command| command.id == id);
        let admitted = match exists {
            Some(command) if matches!(command.state, CommandState::Pending) || cancelled => Ok(id),
            Some(_) => work.app.sessions[origin].retry_command(&id).map(|()| id),
            None => {
                work.app.sessions[origin].submit_wayfinder_command(user_message, request.clone())
            }
        };
        match admitted {
            Ok(id) if cancelled => {
                let reason = "Wayfinder scope operation withdrawn before dispatch";
                work.app.sessions[origin].set_command_state(
                    &id,
                    CommandState::Refused {
                        reason: reason.into(),
                    },
                );
                if pending
                    .as_ref()
                    .is_some_and(|(pending_origin, key)| *pending_origin == origin && key == &id)
                {
                    *pending = None;
                }
                withdraw_wayfinder(work, origin, &request, reason);
            }
            Ok(id) => *pending = Some((origin, id)),
            Err(error) => {
                withdraw_wayfinder(work, origin, &request, &error);
                work.app.notice =
                    format!("Wayfinder intent not saved; no scope action dispatched: {error}");
            }
        }
        changed = true;
    }
    changed
}

pub(crate) fn withdraw_wayfinder(
    work: &mut Workstation,
    origin: usize,
    request: &alfredo_tui::understanding::Request,
    reason: &str,
) {
    let attempt = work
        .wayfinder
        .prepared_origins()
        .into_iter()
        .find(|(session, _, _, candidate)| *session == origin && candidate == request)
        .map(|(_, attempt, _, _)| attempt);
    if work.wayfinder.withdraw_prepared(origin, request) {
        if let Some(attempt) = attempt {
            work.app.sessions[origin]
                .apply(attempt, alfredo_tui::model::Update::Failed(reason.into()));
        }
    }
}

/// Record autopilot's chosen command in an idle conversation, then release it
/// through the same saved-intent barrier as typed commands.
pub(crate) fn submit_autopilot(
    runtime: &Runtime,
    work: &mut Workstation,
    pending: &mut Option<(usize, String)>,
) -> bool {
    let sessions = &work.app.sessions;
    let Some(origin) = std::iter::once(work.app.selected)
        .chain(0..sessions.len())
        .find(|&index| {
            !sessions[index].status.active() && sessions[index].messages.len().is_multiple_of(2)
        })
    else {
        return false;
    };
    let Some(submission) = work.autopilot.tick(runtime, &mut work.tasks) else {
        return false;
    };
    record_autopilot_command(work, origin, submission, pending);
    true
}

/// Record autopilot's command in conversation `origin`. A command that cannot be
/// recorded pauses autopilot with the error as its own notice, so the status panel,
/// the saved state and headless output all name the reason, not just the footer.
pub(crate) fn record_autopilot_command(
    work: &mut Workstation,
    origin: usize,
    submission: alfredo_tui::autopilot::Submission,
    pending: &mut Option<(usize, String)>,
) {
    match work.app.sessions[origin].submit_autopilot_command(submission.text, submission.intent) {
        Ok(id) => {
            work.app.append_history_warning(origin);
            *pending = Some((origin, id));
        }
        Err(error) => {
            let notice = format!("Autopilot paused: command not recorded: {error}");
            work.autopilot
                .pause_because(&mut work.tasks, notice.clone());
            work.app.notice = notice;
        }
    }
}

/// Record an owner instruction's next step in an idle conversation, then release
/// it through the same saved-intent barrier as typed commands.
pub(crate) fn submit_owner(work: &mut Workstation, pending: &mut Option<(usize, String)>) -> bool {
    let sessions = &work.app.sessions;
    let Some(origin) = std::iter::once(work.app.selected)
        .chain(0..sessions.len())
        .find(|&index| {
            !sessions[index].status.active() && sessions[index].messages.len().is_multiple_of(2)
        })
    else {
        return false;
    };
    let Some(submission) =
        alfredo_tui::instruct::Instructions::tick(&mut work.tasks, &mut work.autopilot)
    else {
        return false;
    };
    match work.app.sessions[origin].submit_autopilot_command(submission.text, submission.intent) {
        Ok(id) => {
            work.app.append_history_warning(origin);
            *pending = Some((origin, id));
        }
        Err(error) => work.app.notice = format!("Instruction step not recorded: {error}"),
    }
    true
}

/// An autopilot decision still waiting at the saved-intent barrier for a family
/// (or plan) the owner has since instructed is withdrawn, never dispatched.
pub(crate) fn withdraw_superseded_autopilot(
    work: &mut Workstation,
    pending: &mut Option<(usize, String)>,
) -> bool {
    use alfredo_tui::console_command::CommandState;
    let Some((origin, id)) = pending.as_ref() else {
        return false;
    };
    let superseded = work.app.sessions[*origin]
        .commands()
        .iter()
        .find(|command| command.id == *id)
        .is_some_and(|command| {
            command.text.starts_with("Autopilot · ")
                && matches!(command.state, CommandState::Pending)
                && work
                    .tasks
                    .owner
                    .supersedes(&command.intent, work.tasks.snapshot.as_ref())
        });
    if !superseded {
        return false;
    }
    work.app.sessions[*origin].set_command_state(
        id,
        CommandState::Refused {
            reason: "Withdrawn: your instruction decides for this task".into(),
        },
    );
    *pending = None;
    true
}

/// Locate immutable automatic provenance before admission. Session validation
/// repeats this same-session/earlier-parent requirement before persistence.
pub(crate) fn stage_automatic_command(
    work: &mut Workstation,
    intent: alfredo_tui::command_intent::Intent,
) -> Result<(usize, String), String> {
    use alfredo_tui::{
        command_intent::Intent, console_command::CommandState, control_command::Outcome,
    };
    let parent = work.app.sessions.iter().enumerate().find_map(|(index, session)| {
        session.commands().iter().find_map(|command| {
            let text = match &intent {
                Intent::DispatchRun { request } if matches!(&command.intent, Intent::Control { request: source } if source == &request.source)
                    && matches!(command.state, CommandState::Control { outcome: Outcome::DispatchChanged { enabled: true } }) => {
                    format!("Automatic /run {} · dispatch command #{}", request.task, command.sequence)
                }
                Intent::ArchitectDraft { request } if matches!(&command.intent, Intent::Task { request: source } if source == &request.source) => {
                    let alfredo_tui::planner_command::Operation::Architect { origin, .. } = &request.request.operation else { return None; };
                    format!("Automatic /architect-revise {} · review command #{}", origin.task, command.sequence)
                }
                _ => return None,
            };
            Some((index, text))
        })
    }).ok_or("Saved originating command is unavailable")?;
    let id = work.app.sessions[parent.0].submit_automatic_command(parent.1, intent)?;
    Ok((parent.0, id))
}

/// A saved snapshot alone never authorizes dispatch: a matching live admission
/// token and the current exact Pending record are both required.
pub(crate) fn saved_pending_command(
    app: &alfredo_tui::model::App,
    autosave: &alfredo_tui::conversations::Autosave,
    pending: &Option<(usize, String)>,
) -> Option<(usize, alfredo_tui::console_command::ConsoleCommand)> {
    let (origin, id) = pending.as_ref()?;
    let command = app
        .sessions
        .get(*origin)?
        .commands()
        .iter()
        .find(|command| command.id == *id)?;
    autosave
        .contains_saved_command(*origin, command)
        .then(|| (*origin, command.clone()))
}

/// Withdraw only the automatic draft waiting at the saved-intent barrier. The
/// caller still cancels any independently active planner or retained draft normally.
pub(crate) fn withdraw_pending_architect(
    app: &mut alfredo_tui::model::App,
    pending: &mut Option<(usize, String)>,
) -> bool {
    use alfredo_tui::{command_intent::Intent, console_command::CommandState};
    let Some((origin, id)) = pending.as_ref() else {
        return false;
    };
    let Some(session) = app.sessions.get_mut(*origin) else {
        return false;
    };
    if !session.commands().iter().any(|command| {
        command.id == *id
            && matches!(command.intent, Intent::ArchitectDraft { .. })
            && matches!(command.state, CommandState::Pending)
    }) {
        return false;
    }
    session.set_command_state(
        id,
        CommandState::Refused {
            reason:
                "Automatic Architect draft withdrawn for plan cancellation; generation not started"
                    .into(),
        },
    );
    *pending = None;
    true
}

/// Esc, and Ctrl+C during a running chat turn (issue #345): a pending action is denied
/// (logged as cancelled), the request is aborted and the turn ends; the draft is kept.
pub(crate) fn cancel_chat_turn(
    app: &mut alfredo_tui::model::App,
    index: usize,
    jobs: &mut [Option<JoinHandle<()>>],
    approval_replies: &mut std::collections::HashMap<
        usize,
        alfredo_tui::chat_tools::approval::Request,
    >,
) {
    if let Some(request) = approval_replies.remove(&index) {
        request.cancel();
    }
    if let Some(job) = jobs[index].take() {
        job.abort();
    }
    let summarising = app.sessions[index].compacting();
    let held = !app.sessions[index].queued_messages().is_empty();
    app.sessions[index].cancel();
    if held {
        app.notice = "Queued text is back in the prompt; nothing was sent".into();
    }
    if summarising {
        app.notice = "Summary cancelled; your prompt is kept in the draft".into();
    }
}
