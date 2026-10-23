use super::{
    dim, receipt_style, safe, single_line, truncate, verified_receipt, wrap_words, Color, Line,
    Modifier, Span, Style, COMMAND_SUMMARY,
};

/// One short human sentence per canonical receipt.
pub fn short_phase(receipt: &crate::tasks::Receipt) -> String {
    use crate::tasks::{Action, TaskStatus};
    let id = receipt.task;
    match &receipt.request.action {
        Action::Propose { .. } => format!("Task #{id} proposed"),
        Action::Plan { plan } => match plan.tasks.len() {
            0 | 1 => format!("Plan saved · task #{id}"),
            count => format!(
                "Plan saved · {count} tasks #{id}–#{}",
                id + count as u64 - 1
            ),
        },
        Action::Assign { .. } => format!("Task #{id} worker assigned · needs approval"),
        Action::Permit { .. } => format!("Task #{id} files and check set · needs approval"),
        Action::Approve { .. } => format!("Task #{id} approved"),
        Action::Cancel { .. } => format!("Task #{id} cancelled"),
        Action::Requeue { .. } => format!("Task #{id} requeued"),
        Action::Start { .. } => format!("Task #{id} started"),
        Action::Finish { status, .. } => match status {
            TaskStatus::ReviewReady => format!("Task #{id} check passed · awaiting review"),
            TaskStatus::Failed => format!("Task #{id} failed"),
            TaskStatus::Cancelled => format!("Task #{id} run cancelled"),
            TaskStatus::NeedsHumanReview => format!("Task #{id} held for human review"),
            other => format!("Task #{id} finished · {other:?}"),
        },
        Action::Repair { task, .. } => format!("Repair #{id} proposed for task #{task}"),
        Action::Branch { .. } => format!("Task #{id} review branch saved"),
        Action::ResolveRepair { .. } => format!("Repair #{id} resolves its task"),
        Action::ReviewArchitecture { task, .. } if *task == id => {
            format!("Task #{id} needs an architect revision")
        }
        Action::ReviewArchitecture { .. } => format!("Architecture repair #{id} proposed"),
        Action::ReviewAndRepair { task, decision } => format!(
            "Task #{task} review: {} · repair #{id} proposed",
            decision.outcome.label()
        ),
        Action::Decide { decision, .. } => decision.risk.map_or_else(
            || format!("Task #{id} review: {}", decision.outcome.label()),
            |risk| format!("Task #{id} held · {} risk needs human review", risk.label()),
        ),
        Action::Assess { assessment, .. } => format!(
            "Task #{id} {}",
            if assessment.accept {
                "accepted"
            } else {
                "rejected"
            }
        ),
        Action::Review { accept, .. } => format!(
            "Task #{id} {}",
            if *accept { "accepted" } else { "rejected" }
        ),
    }
}

/// How one collapsed step reads.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StepKind {
    Done,
    Failed,
    Stopped,
    Pending,
    /// A repair was proposed; the line's glyph is its task's own outcome.
    Repair,
}
pub(super) type Phase = (String, StepKind);

pub(super) struct Step {
    pub(super) task: Option<u64>,
    /// Empty: nothing worth a line (such as an acknowledged `/dispatch on`).
    pub(super) phases: Vec<Phase>,
}

/// One word for what a canonical receipt did to its task.
fn phase_word(receipt: &crate::tasks::Receipt) -> Vec<Phase> {
    use crate::assessment::Outcome;
    use crate::tasks::{Action, TaskStatus};
    let done = |text: &str| vec![(text.to_owned(), StepKind::Done)];
    let repair = |reason: &str| {
        if reason.starts_with(crate::instruct::OWNER) {
            format!("repair #{} with your note", receipt.task)
        } else {
            format!("repair #{}", receipt.task)
        }
    };
    let review = |outcome: Outcome, held: bool| {
        if held {
            vec![("held for review".to_owned(), StepKind::Stopped)]
        } else if outcome.approves() {
            done("accepted")
        } else if outcome == Outcome::NeedsRepair {
            vec![("needs repair".to_owned(), StepKind::Failed)]
        } else {
            vec![("rejected".to_owned(), StepKind::Failed)]
        }
    };
    match &receipt.request.action {
        Action::Plan { .. } => done("planned"),
        Action::Propose { .. } => done("proposed"),
        Action::Permit { .. } => done("files and check set"),
        Action::Approve { .. } => done("approved"),
        Action::Requeue { .. } => done("requeued"),
        Action::Assign { .. } => done("assigned"),
        Action::Start { .. } => done("started"),
        Action::Finish { status, .. } => match status {
            TaskStatus::ReviewReady => done("check passed"),
            TaskStatus::Cancelled => vec![("cancelled".to_owned(), StepKind::Stopped)],
            _ => vec![("failed".to_owned(), StepKind::Failed)],
        },
        Action::Cancel { .. } => vec![("cancelled".to_owned(), StepKind::Stopped)],
        Action::Repair { reason, .. } => vec![(repair(reason), StepKind::Repair)],
        Action::ReviewAndRepair { decision, .. } => {
            let mut phases = review(decision.outcome, false);
            phases.push((repair(&decision.reason), StepKind::Repair));
            phases
        }
        Action::ReviewArchitecture { .. } => {
            vec![("architect revision".to_owned(), StepKind::Stopped)]
        }
        Action::Decide { decision, .. } => {
            review(decision.outcome, decision.requires_human_review())
        }
        Action::Assess { assessment, .. } => {
            if assessment.accept {
                done("accepted")
            } else {
                vec![("rejected".to_owned(), StepKind::Failed)]
            }
        }
        Action::Review { accept, .. } => {
            if *accept {
                done("accepted")
            } else {
                vec![("rejected".to_owned(), StepKind::Failed)]
            }
        }
        Action::ResolveRepair { .. } => done("resolved"),
        Action::Branch { .. } => done("branch saved"),
    }
}

/// Automatic commands (autopilot's, owner-instruction steps and launches) read
/// as short steps; typed commands keep their full entry. Detail stays in F4.
pub(super) fn collapsed_step(
    command: &crate::console_command::ConsoleCommand,
    all: &[crate::console_command::ConsoleCommand],
    tasks: Option<&crate::task_control::TaskControl>,
) -> Option<Step> {
    use crate::command_intent::{Acknowledgment, Intent};
    use crate::console_command::CommandState;
    use crate::tasks::Action;
    // A launch collapses when autopilot turned dispatch on for it.
    let automatic = command.text.starts_with("Autopilot · ")
        || command.text.starts_with("You · ")
        || matches!(&command.intent, Intent::DispatchRun { request } if all.iter().any(|parent| {
            parent.text.starts_with("Autopilot · ")
                && matches!(&parent.intent, Intent::Control { request: source } if *source == request.source)
        }));
    if !automatic {
        return None;
    }
    let snapshot = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    let receipt = |acknowledgment: Acknowledgment| match acknowledgment {
        Acknowledgment::Task {
            revision,
            task,
            correlation,
        } => verified_receipt(snapshot, revision, task, &correlation).cloned(),
        Acknowledgment::Scope { .. } => None,
    };
    let unsettled = || -> Vec<Phase> {
        match &command.state {
            CommandState::Refused { reason } if reason.starts_with("Withdrawn") => {
                vec![("withdrawn".into(), StepKind::Stopped)]
            }
            CommandState::Refused { reason } => {
                vec![(
                    format!("refused: {}", single_line(reason)),
                    StepKind::Failed,
                )]
            }
            CommandState::Unknown { reason } => vec![(
                format!("unconfirmed: {}", single_line(reason)),
                StepKind::Pending,
            )],
            _ => vec![("…".into(), StepKind::Pending)],
        }
    };
    let task_of = |action: &Action| match action {
        Action::Approve { task }
        | Action::Cancel { task }
        | Action::Requeue { task }
        | Action::Permit { task, .. }
        | Action::Assign { task, .. }
        | Action::Repair { task, .. }
        | Action::ReviewAndRepair { task, .. }
        | Action::ReviewArchitecture { task, .. }
        | Action::Decide { task, .. }
        | Action::Assess { task, .. }
        | Action::Review { task, .. }
        | Action::ResolveRepair { task }
        | Action::Branch { task, .. } => Some(*task),
        _ => None,
    };
    Some(match &command.intent {
        Intent::DispatchRun { .. } | Intent::Run { .. } => {
            let task = match &command.intent {
                Intent::DispatchRun { request } => request.task,
                Intent::Run { task, .. } => *task,
                _ => unreachable!(),
            };
            let receipts: Vec<_> = command
                .intent
                .task_receipts(snapshot)
                .into_iter()
                .filter_map(receipt)
                .collect();
            let mut phases: Vec<Phase> = receipts.iter().flat_map(phase_word).collect();
            if phases.is_empty() {
                phases = unsettled();
            } else if receipts.len() == 1 {
                phases.push(("…".into(), StepKind::Pending));
            }
            Step {
                task: Some(task),
                phases,
            }
        }
        Intent::Control { request } => match &request.operation {
            crate::control_command::Operation::CancelWorker { task, .. } => Step {
                task: Some(*task),
                phases: match &command.state {
                    CommandState::Control { .. } => vec![],
                    _ => unsettled(),
                },
            },
            crate::control_command::Operation::Dispatch { .. } => Step {
                task: None,
                phases: match &command.state {
                    CommandState::Control { .. } => vec![],
                    _ => unsettled()
                        .into_iter()
                        .map(|(text, kind)| (format!("dispatch {text}"), kind))
                        .collect(),
                },
            },
        },
        Intent::Planner { request } => {
            let revise = matches!(
                request.operation,
                crate::planner_command::Operation::Revise { .. }
            );
            Step {
                task: None,
                phases: match &command.state {
                    CommandState::Planner { outcome } => vec![match outcome {
                        crate::planner_command::Outcome::Generated { .. } if revise => {
                            ("plan revised".into(), StepKind::Done)
                        }
                        crate::planner_command::Outcome::Generated { .. } => {
                            ("plan drafted".into(), StepKind::Done)
                        }
                        crate::planner_command::Outcome::Stopped => {
                            ("plan stopped".into(), StepKind::Stopped)
                        }
                        crate::planner_command::Outcome::Failed { reason } => (
                            format!(
                                "plan failed: {}",
                                single_line(&crate::planner::plain_error(reason))
                            ),
                            StepKind::Failed,
                        ),
                    }],
                    CommandState::Submitted | CommandState::Pending => {
                        vec![("planning …".into(), StepKind::Pending)]
                    }
                    _ => unsettled(),
                },
            }
        }
        Intent::Task { request } => {
            let acknowledged = command.intent.reconcile(snapshot, None).and_then(receipt);
            let task = match &request.action {
                Action::Plan { .. } | Action::Propose { .. } => {
                    acknowledged.as_ref().map(|receipt| receipt.task)
                }
                action => task_of(action),
            };
            Step {
                task,
                phases: acknowledged
                    .as_ref()
                    .map(phase_word)
                    .unwrap_or_else(unsettled),
            }
        }
        Intent::Scope { .. } => Step {
            task: None,
            phases: match command.intent.reconcile(
                snapshot,
                tasks.and_then(|tasks| tasks.canonical_scope.as_ref()),
            ) {
                Some(_) => vec![("scope saved".into(), StepKind::Done)],
                None => unsettled(),
            },
        },
        _ => return None,
    })
}

/// The same failure line twice in a row (a planner retried by autopilot)
/// becomes one line with a count: `plan failed: … ×3`.
pub(super) fn repeated_failure(previous: &[Phase], next: &[Phase]) -> Option<Vec<Phase>> {
    let ([(text, StepKind::Failed)], [(again, StepKind::Failed)]) = (previous, next) else {
        return None;
    };
    let (base, count) = match text.rsplit_once(" ×") {
        Some((base, count)) => (base, count.parse::<u32>().ok()?),
        None => (text.as_str(), 1),
    };
    (base == again).then(|| vec![(format!("{base} ×{}", count + 1), StepKind::Failed)])
}

/// One collapsed entry, wrapped at word boundaries with a hanging indent.
pub(super) fn collapsed_lines(
    task: Option<u64>,
    phases: &[Phase],
    width: usize,
) -> Vec<Line<'static>> {
    let line = collapsed_line(task, phases);
    let glyph = line.spans[0].clone();
    let text: String = line.spans[1].content.trim_start().to_owned();
    let indent = 2;
    wrap_words(&text, width.saturating_sub(indent))
        .into_iter()
        .enumerate()
        .map(|(row, chunk)| {
            if row == 0 {
                Line::from(vec![glyph.clone(), Span::raw(format!(" {chunk}"))])
            } else {
                Line::from(format!("{}{chunk}", " ".repeat(indent)))
            }
        })
        .collect()
}

fn collapsed_line(task: Option<u64>, phases: &[Phase]) -> Line<'static> {
    let outcome = phases
        .iter()
        .rev()
        .find(|(_, kind)| *kind != StepKind::Repair)
        .map_or(StepKind::Done, |(_, kind)| *kind);
    let (glyph, color) = match outcome {
        StepKind::Done | StepKind::Repair => ("✓", Color::Green),
        StepKind::Failed => ("✗", Color::Red),
        StepKind::Stopped => ("–", Color::Yellow),
        StepKind::Pending => ("…", Color::Yellow),
    };
    let text = phases
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join(" → ");
    let text = match task {
        Some(task) => format!(" #{task} {text}"),
        None => format!(" {text}"),
    };
    Line::from(vec![
        Span::styled(glyph, Style::default().fg(color)),
        Span::raw(safe(&text)),
    ])
}

/// Transcript entry for a saved command: what was asked, then one short outcome line.
/// Correlations, receipt revisions and command sequence numbers are not shown here.
pub(super) fn command_lines(
    command: &crate::console_command::ConsoleCommand,
    tasks: Option<&crate::task_control::TaskControl>,
) -> Vec<Line<'static>> {
    use crate::command_intent::Intent;
    use crate::console_command::CommandState;
    let snapshot = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    if let (Intent::SelectionArrival { request }, CommandState::Selection { outcome }) =
        (&command.intent, &command.state)
    {
        let ready = matches!(
            outcome.phase,
            crate::selection_command::Phase::Selected
                | crate::selection_command::Phase::AlreadyCurrent
        );
        let (phase, style) = if outcome.failure.is_some() {
            (
                selection_phase(request, outcome),
                Style::default().fg(Color::Red),
            )
        } else if ready {
            ("ready".to_owned(), dim())
        } else {
            (selection_phase(request, outcome), dim())
        };
        return vec![
            Line::styled(format!("· {} · {phase}", single_line(&command.text)), style),
            Line::default(),
        ];
    }
    let automatic = matches!(command.intent, Intent::DispatchRun { .. });
    let selection = command.intent.selection_request();
    let run = automatic || matches!(command.intent, Intent::Run { .. });
    let cancel_worker = matches!(&command.intent, Intent::Control { request }
        if matches!(request.operation, crate::control_command::Operation::CancelWorker { .. }));
    let mut lifecycle = if run || cancel_worker {
        command.intent.task_receipts(snapshot).into_iter()
    } else {
        Vec::new().into_iter()
    };
    let acknowledgment = if run {
        lifecycle.next()
    } else {
        command.intent.reconcile(
            snapshot,
            tasks.and_then(|tasks| tasks.canonical_scope.as_ref()),
        )
    }
    .map(|acknowledgment| command_acknowledgment(acknowledgment, tasks));
    if cancel_worker {
        // The Start establishes the target; it does not acknowledge requesting cancellation.
        lifecycle.next();
    }
    let (phase, color) = match acknowledgment {
        Some((text, glyph, color)) => (format!("{glyph} {text}"), color),
        None => match &command.state {
            CommandState::Pending => ("… Pending · saving intent".into(), Color::Yellow),
            CommandState::Submitted if command.intent.planner_request().is_some() => (
                "… Submitted · planner operation pending".into(),
                Color::Yellow,
            ),
            CommandState::Submitted if matches!(command.intent, Intent::Control { .. }) => (
                "… Submitted · controller operation pending".into(),
                Color::Yellow,
            ),
            CommandState::Submitted if matches!(command.intent, Intent::Wayfinder { .. }) => {
                ("… Submitted · awaiting scope receipt".into(), Color::Yellow)
            }
            CommandState::Submitted if selection.is_some() => {
                ("… Submitted · preparing selection".into(), Color::Yellow)
            }
            CommandState::Submitted => (
                "… Submitted · awaiting acknowledgment".into(),
                Color::Yellow,
            ),
            CommandState::Unknown { reason } => {
                (format!("? Outcome unconfirmed · {reason}"), Color::Yellow)
            }
            CommandState::Refused { reason } => {
                (format!("✗ Not dispatched · {reason}"), Color::Red)
            }
            CommandState::Selection { outcome } => (
                selection
                    .map(|request| selection_phase(request, outcome))
                    .unwrap_or_else(|| "Selection observation unavailable".into()),
                if outcome.failure.is_some() {
                    Color::Red
                } else {
                    Color::Reset
                },
            ),
            CommandState::Control { outcome } => match outcome {
                crate::control_command::Outcome::CancellationRequested => {
                    ("✓ Cancellation requested".into(), Color::Green)
                }
                crate::control_command::Outcome::DispatchChanged { enabled } => (
                    format!("✓ Dispatch {}", if *enabled { "on" } else { "off" }),
                    Color::Green,
                ),
            },
            CommandState::Planner { outcome } => match outcome {
                crate::planner_command::Outcome::Generated { tasks, .. } => (
                    format!(
                        "✓ Draft generated · {tasks} {} · saving and approval are separate",
                        if *tasks == 1 { "step" } else { "steps" }
                    ),
                    Color::Green,
                ),
                crate::planner_command::Outcome::Stopped
                    if matches!(&command.intent, Intent::Planner { request }
                        if matches!(request.operation, crate::planner_command::Operation::Cancel { generation: None, .. })) =>
                {
                    ("– Draft discarded".into(), Color::Yellow)
                }
                crate::planner_command::Outcome::Stopped => {
                    ("– Draft generation stopped".into(), Color::Yellow)
                }
                crate::planner_command::Outcome::Failed { reason } => (
                    if matches!(&command.intent, Intent::Planner { request }
                        if matches!(request.operation, crate::planner_command::Operation::Cancel { .. }))
                    {
                        format!("✗ Planner cancellation failed · {reason}")
                    } else {
                        format!("✗ Draft generation failed · {reason}")
                    },
                    Color::Red,
                ),
            },
        },
    };
    let heading = match &command.intent {
        Intent::DispatchRun { request } => format!("▶ Dispatch · run task #{}", request.task),
        Intent::ArchitectDraft { request } => match &request.request.operation {
            crate::planner_command::Operation::Architect { origin, .. } => {
                format!("◆ Architect · revise task #{}", origin.task)
            }
            _ => format!("◆ Architect · {}", single_line(&command.text)),
        },
        Intent::Wayfinder { user_message, .. } => {
            format!("◇ Wayfinder · turn {}", user_message / 2 + 1)
        }
        _ if selection.is_some() => format!("› {}", single_line(&command.text)),
        _ => {
            let text = safe(&command.text);
            let mut parts = text.lines();
            // Automatic repair commands carry whole check output; the task
            // detail and F3 evidence hold it, the transcript keeps a summary.
            let first = parts.next().unwrap_or_default();
            let first = if command.text.starts_with("Autopilot") {
                truncate(first, COMMAND_SUMMARY)
            } else {
                first.to_owned()
            };
            let rest: Vec<String> = parts.map(str::to_owned).collect();
            let mut lines = vec![Line::styled(
                format!("› {first}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )];
            lines.extend(rest.into_iter().map(Line::from));
            return finish_command_lines(
                lines,
                phase,
                color,
                run || cancel_worker,
                lifecycle,
                tasks,
            );
        }
    };
    let mut lines = vec![Line::styled(
        heading,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    if matches!(command.intent, Intent::Wayfinder { .. }) {
        lines.push(Line::from(single_line(&command.text)));
    }
    finish_command_lines(lines, phase, color, run || cancel_worker, lifecycle, tasks)
}

fn finish_command_lines(
    mut lines: Vec<Line<'static>>,
    phase: String,
    color: Color,
    lifecycle_result: bool,
    mut lifecycle: std::vec::IntoIter<crate::command_intent::Acknowledgment>,
    tasks: Option<&crate::task_control::TaskControl>,
) -> Vec<Line<'static>> {
    lines.push(Line::styled(
        single_line(&phase),
        Style::default().fg(color),
    ));
    if lifecycle_result {
        match lifecycle
            .next()
            .map(|acknowledgment| command_acknowledgment(acknowledgment, tasks))
        {
            Some((text, glyph, color)) => lines.push(Line::styled(
                format!("{glyph} {text}"),
                Style::default().fg(color),
            )),
            None => lines.push(Line::styled(
                "… no result yet",
                Style::default().fg(Color::Yellow),
            )),
        }
    }
    lines.push(Line::default());
    lines
}

/// Selection observations describe completed preparation milestones. They are
/// independent of task approval and never imply a swap from preparation alone.
fn selection_phase(
    request: &crate::selection_command::Request,
    outcome: &crate::selection_command::Outcome,
) -> String {
    use crate::selection_command::{MissionChoice, Phase, WorkspaceChoice};
    let repository = match request.choice.workspace {
        WorkspaceChoice::Existing { .. } => "Repository opened",
        WorkspaceChoice::Create { .. } => "Repository created",
    };
    let mission = match request.choice.mission {
        MissionChoice::Resume { .. } => "Mission resumed",
        MissionChoice::StartNew { .. } => "Mission created",
    };
    let phase = match outcome.phase {
        Phase::Admitted => "Selection admitted · repository pending".into(),
        Phase::RepositoryReady => format!("{repository} · mission pending"),
        Phase::MissionReady => format!("{repository} · {mission} · target pending"),
        Phase::TargetLoaded => {
            format!("{repository} · {mission} · target loaded · handoff pending")
        }
        Phase::HandoffPrepared => format!(
            "{repository} · {mission} · target loaded · handoff prepared · selection not recorded"
        ),
        Phase::Selected => format!("{repository} · {mission} · target loaded · workspace selected"),
        Phase::AlreadyCurrent => "Already current · no handoff needed".into(),
    };
    match &outcome.failure {
        Some(reason) => format!("{phase} · Failed: {reason}"),
        None => phase,
    }
}

/// Short outcome text, glyph and colour for an acknowledged command.
fn command_acknowledgment(
    acknowledgment: crate::command_intent::Acknowledgment,
    tasks: Option<&crate::task_control::TaskControl>,
) -> (String, &'static str, Color) {
    match acknowledgment {
        crate::command_intent::Acknowledgment::Task {
            revision,
            task,
            correlation,
        } => match verified_receipt(
            tasks.and_then(|tasks| tasks.snapshot.as_ref()),
            revision,
            task,
            &correlation,
        ) {
            Some(receipt) => {
                let (glyph, color) = receipt_style(receipt);
                (short_phase(receipt), glyph, color)
            }
            None => (format!("Task #{task} updated"), "✓", Color::Green),
        },
        crate::command_intent::Acknowledgment::Scope { .. } => {
            ("Scope saved".into(), "✓", Color::Green)
        }
    }
}
