use super::{
    dim, frame_block, safe, short_phase, side_pane, single_line, task_state_word, truncate,
    wrap_words, Clear, Color, Constraint, Frame, Instant, Layout, Line, Modifier, Paragraph, Rect,
    Span, Style, TaskControl, Theme, Tone, UnicodeWidthStr, Wrap, GAP, LABEL, MAX_DEPTH,
};

/// `Label    value`: dim label column, value wrapped with a hanging indent.
fn labeled(label: &str, value: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let room = width.saturating_sub(LABEL).max(12);
    let mut lines = Vec::new();
    for chunk in safe(value).lines().flat_map(|line| wrap_words(line, room)) {
        let head = if lines.is_empty() {
            format!("{label:<LABEL$}")
        } else {
            " ".repeat(LABEL)
        };
        lines.push(Line::from(vec![
            Span::styled(head, dim()),
            Span::styled(chunk, style),
        ]));
    }
    if lines.is_empty() {
        lines.push(Line::styled(format!("{label:<LABEL$}"), dim()));
    }
    lines
}

/// Review summary without its label prefix; identical per-criterion notes
/// (such as autopilot's) collapse into one line.
fn compact_review(summary: &str) -> String {
    let mut lines = Vec::new();
    let mut criteria: Vec<(&str, &str)> = Vec::new();
    for line in summary.lines() {
        if let Some((number, note)) = line
            .strip_prefix("Criterion ")
            .and_then(|rest| rest.split_once(" · "))
        {
            criteria.push((number, note));
            continue;
        }
        lines.push(
            line.strip_prefix("Reviewer outcome: ")
                .unwrap_or(line)
                .to_owned(),
        );
    }
    match criteria.as_slice() {
        [] => {}
        [(first, note), .., (last, _)] if criteria.iter().all(|(_, other)| other == note) => {
            lines.push(format!("Criteria {first}–{last} · {note}"))
        }
        _ => lines.extend(
            criteria
                .iter()
                .map(|(number, note)| format!("Criterion {number} · {note}")),
        ),
    }
    lines.join("\n")
}

/// Readiness without embedded command instructions, or nothing when it only
/// restates the status line.
fn plain_readiness(text: &str) -> String {
    const RESTATED: [&str; 8] = [
        "Failed;",
        "Rejected;",
        "Cancelled;",
        "Check passed;",
        "Held for human review;",
        "Accepted result",
        "Repair requested;",
        "Run claimed",
    ];
    if RESTATED.iter().any(|prefix| text.starts_with(prefix)) {
        return String::new();
    }
    text.split(" · ")
        .filter(|part| !part.trim_start().starts_with('/'))
        .map(|part| {
            part.split_once("; /")
                .map_or(part, |(before, _)| before)
                .trim()
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn status_word(snapshot: &crate::tasks::Snapshot, task: &crate::tasks::Task) -> &'static str {
    use crate::tasks::TaskStatus;
    let decided = |outcome| {
        snapshot
            .decision_for_task(task.id)
            .is_some_and(|decision| decision.outcome == outcome)
    };
    match task.status {
        TaskStatus::Proposed => "Needs approval",
        // A pre-policy save can hold an approval with no policy; never call it approved.
        TaskStatus::Approved if task.policy.is_none() => "Needs files/check policy",
        TaskStatus::Approved => "Approved",
        TaskStatus::Cancelled => "Cancelled",
        TaskStatus::Running => "Running",
        TaskStatus::NeedsHumanReview => "Held for human review",
        TaskStatus::ReviewReady => "Check passed · needs review",
        TaskStatus::Accepted if decided(crate::assessment::Outcome::ApprovedWithLimitations) => {
            "Accepted with limitations"
        }
        TaskStatus::Rejected if decided(crate::assessment::Outcome::NeedsRepair) => "Needs repair",
        TaskStatus::Accepted => "Accepted",
        TaskStatus::Failed => "Failed",
        TaskStatus::Rejected => "Rejected",
    }
}

/// The command a user needs next, only when the task waits for a decision.
pub(super) fn next_action(
    snapshot: &crate::tasks::Snapshot,
    task: &crate::tasks::Task,
) -> Option<String> {
    use crate::tasks::TaskStatus;
    let id = task.id;
    match task.status {
        TaskStatus::Proposed if task.policy.is_none() => Some(format!("/permit {id} JSON")),
        TaskStatus::Proposed => Some(format!("/approve {id}")),
        TaskStatus::Approved if task.policy.is_none() => Some(format!("/permit {id} JSON")),
        TaskStatus::Approved => Some(format!("/run {id}")),
        TaskStatus::ReviewReady if snapshot.acceptance_for_task(id).is_empty() => {
            Some(format!("/accept {id}{GAP}/reject {id}"))
        }
        TaskStatus::ReviewReady => Some(format!("/review {id} JSON{GAP}/reject {id}")),
        TaskStatus::NeedsHumanReview => Some(format!("/review {id} JSON")),
        TaskStatus::Failed
            if task
                .run
                .as_ref()
                .is_some_and(|run| crate::worker::too_large_advice(&run.detail).is_some()) =>
        {
            // A repair cannot shrink the file; say what can, as autopilot's summary does.
            task.run
                .as_ref()
                .and_then(|run| crate::worker::too_large_advice(&run.detail))
                .map(|(_, next)| next.to_string())
        }
        TaskStatus::Failed
            if task.run.as_ref().is_some_and(|run| {
                crate::worker::model_unreachable_advice(&run.detail).is_some()
            }) =>
        {
            // A repair of the code cannot start the server; show the way forward.
            task.run
                .as_ref()
                .and_then(|run| crate::worker::model_unreachable_advice(&run.detail))
                .map(|(_, next)| next.to_string())
        }
        TaskStatus::Rejected | TaskStatus::Failed if snapshot.architecture_required(id) => {
            Some(format!("/architect-revise {id}"))
        }
        TaskStatus::Rejected | TaskStatus::Failed
            if snapshot.resolved_by(id).is_none()
                && !snapshot.architecture_obsolete(id)
                && !snapshot
                    .tasks
                    .iter()
                    .any(|child| child.repair_of == Some(id)) =>
        {
            Some(format!("/repair {id} REASON"))
        }
        TaskStatus::Accepted
            if task.repair_of.is_some() && snapshot.resolution_for_family(id).is_none() =>
        {
            Some(format!("/resolve-repair {id}"))
        }
        _ => None,
    }
}

fn heading(text: &str) -> Line<'static> {
    Line::styled(
        text.to_owned(),
        Style::default().add_modifier(Modifier::BOLD),
    )
}

/// Right pane detail. Task: glyph, id and title; status line; labeled facts;
/// result and diff; live output while a worker runs. Group: goal, progress and
/// tasks. Keys live in the footer and F1; receipts stay in F3/F4.
pub(super) fn work_inspector(
    tasks: &TaskControl,
    tree: &crate::mission_work::Tree,
    focused: Option<&crate::mission_work::Row>,
    width: usize,
    theme: &Theme,
    now: Instant,
) -> (Vec<Line<'static>>, String, bool, usize) {
    let (lines, title, live, pinned) = work_detail(tasks, tree, focused, width, theme, now);
    // The filter shows only while active, in the title so it costs no row.
    let title = if tasks.task_query.trim().is_empty() {
        title
    } else {
        format!(
            "{}{GAP}filter {}   {}/{} tasks ",
            title.trim_end(),
            single_line(&tasks.task_query),
            tree.matched_tasks,
            tree.total_tasks
        )
    };
    (lines, title, live, pinned)
}

fn work_detail(
    tasks: &TaskControl,
    tree: &crate::mission_work::Tree,
    focused: Option<&crate::mission_work::Row>,
    width: usize,
    theme: &Theme,
    now: Instant,
) -> (Vec<Line<'static>>, String, bool, usize) {
    use crate::mission_work::NodeId;
    use crate::tasks::TaskStatus;
    let Some(snapshot) = tasks.snapshot.as_ref() else {
        return (
            vec![Line::from("Task state unavailable; /refresh to retry")],
            " Tasks ".into(),
            false,
            0,
        );
    };
    let normal = Style::default();
    let mut lines = Vec::new();
    let Some(row) = focused else {
        if snapshot.tasks.is_empty() {
            lines.push(Line::from("No tasks proposed yet."));
            lines.push(Line::styled("/go GOAL plans and runs work.", dim()));
        } else if tree.rows.is_empty() {
            lines.push(Line::from("No matching tasks."));
        } else {
            lines.push(Line::from("Nothing selected."));
        }
        return (lines, " Tasks ".into(), false, 0);
    };
    let task_glyph = |task: &crate::tasks::Task| {
        let status = side_pane::task_status(snapshot, task, tasks);
        Span::styled(
            theme.status_glyph(status, now),
            Style::default().fg(theme.color(Theme::tone(status))),
        )
    };
    let Some(task) = row
        .task
        .and_then(|id| snapshot.tasks.iter().find(|task| task.id == id))
    else {
        let goal = match row.id {
            NodeId::Plan(_) => row
                .label
                .split_once(" · ")
                .map_or(row.label.as_str(), |(_, goal)| goal),
            _ => "Manual tasks",
        };
        for chunk in wrap_words(&single_line(goal), width) {
            lines.push(Line::styled(
                chunk,
                Style::default().add_modifier(Modifier::BOLD),
            ));
        }
        lines.push(Line::default());
        // Every member, regardless of folding or filtering.
        let full = crate::mission_work::project(
            snapshot,
            &tasks.scope_status,
            "",
            &std::collections::BTreeSet::new(),
        );
        let members: Vec<_> = full
            .rows
            .iter()
            .skip_while(|member| member.id != row.id)
            .skip(1)
            .take_while(|member| member.depth > 0)
            .filter_map(|member| {
                let task = snapshot
                    .tasks
                    .iter()
                    .find(|task| Some(task.id) == member.task)?;
                Some((member.depth, task))
            })
            .collect();
        let originals: Vec<_> = members
            .iter()
            .filter(|(_, task)| task.repair_of.is_none())
            .collect();
        let done = originals
            .iter()
            .filter(|(_, task)| {
                task.status == TaskStatus::Accepted
                    || snapshot.resolution_for_family(task.id).is_some()
            })
            .count();
        let repairs = members.len() - originals.len();
        let mut progress = format!("{done}/{} done", originals.len());
        if repairs > 0 {
            progress.push_str(&format!(
                "{GAP}{repairs} repair{}",
                if repairs == 1 { "" } else { "s" }
            ));
        }
        lines.push(Line::from(progress));
        lines.push(Line::default());
        for (depth, task) in members {
            // One line per task; the task detail has the whole title.
            let indent = "  ".repeat(depth.saturating_sub(1).min(MAX_DEPTH - 1));
            let text = truncate(
                &format!(" #{} {}", task.id, single_line(&task.title)),
                width.saturating_sub(indent.width() + 1),
            );
            lines.push(Line::from(vec![
                Span::raw(indent),
                task_glyph(task),
                Span::raw(text),
            ]));
        }
        return (lines, " Group ".into(), false, 0);
    };
    let status = side_pane::task_status(snapshot, task, tasks);
    let color = theme.color(Theme::tone(status));
    let head = format!("{} #{}  ", theme.status_glyph(status, now), task.id);
    // At most two title lines; long repair titles carry whole check output.
    let title_width = width.saturating_sub(head.width());
    let mut title = wrap_words(&single_line(&task.title), title_width);
    if title.len() > 2 {
        title.truncate(2);
        title[1] = truncate(&format!("{} …", title[1]), title_width);
    }
    for (index, chunk) in title.into_iter().enumerate() {
        lines.push(Line::from(vec![
            if index == 0 {
                Span::styled(
                    head.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(" ".repeat(head.width()))
            },
            Span::styled(chunk, Style::default().add_modifier(Modifier::BOLD)),
        ]));
    }
    let stage = tasks.worker_stage(task.id);
    let mut facts = format!(" · {}", single_line(&task.model));
    if let Some((_, elapsed, _)) = stage {
        facts.push_str(&format!(" · {}", side_pane::elapsed(elapsed)));
    }
    lines.push(Line::from(vec![
        Span::styled(status_word(snapshot, task), Style::default().fg(color)),
        Span::styled(facts, dim()),
    ]));
    lines.push(Line::default());
    let live = tasks.worker_live(task.id);
    let amber = Style::default().fg(theme.color(Tone::Amber));
    if let Some(live) = &live {
        // `STAGE · 3.6s · 757 B received`: elapsed is already on the status line.
        let stage = safe(&live.stage);
        let seconds = |part: &str| {
            part.strip_suffix('s')
                .is_some_and(|number| number.parse::<f64>().is_ok())
        };
        let mut text = stage
            .split(" · ")
            .filter(|part| !seconds(part))
            .collect::<Vec<_>>()
            .join(GAP);
        if live.cancelling {
            text = format!("cancellation requested{GAP}{text}");
        }
        lines.extend(labeled("Stage", &text, width, amber));
    }
    let pinned = if live.is_some() { lines.len() } else { 0 };
    if let Some(policy) = &task.policy {
        lines.extend(labeled("Files", &policy.files.join(", "), width, normal));
        lines.extend(labeled("Check", &policy.check.join(" "), width, normal));
    }
    if !task.dependencies.is_empty() {
        let text = task
            .dependencies
            .iter()
            .map(|id| match snapshot.dependency_source(*id) {
                Ok(source) if source.id != *id => format!("#{id} via repair #{}", source.id),
                _ => format!("#{id}"),
            })
            .collect::<Vec<_>>()
            .join(", ");
        lines.extend(labeled("Depends", &text, width, normal));
    }
    if let Some(parent) = task.repair_of {
        lines.extend(labeled("Repair", &format!("of #{parent}"), width, normal));
    }
    if !matches!(task.status, TaskStatus::Running)
        && (task.status != TaskStatus::Accepted || task.repair_of.is_some())
    {
        let text = plain_readiness(&tasks.scope_status.readiness(snapshot, task));
        if !text.is_empty() {
            lines.extend(labeled("State", &text, width, amber));
        }
    }
    if let Some(error) = tasks.dispatch.failures.get(&task.id) {
        lines.extend(labeled(
            "Error",
            &format!("start paused: {}", single_line(error)),
            width,
            Style::default().fg(theme.color(Tone::Red)),
        ));
    }
    // A run blocker (such as pending scope) names its own command first.
    let mut actions: Vec<String> = tasks
        .scope_status
        .run_blocker(snapshot, task)
        .map(|blocker| {
            blocker
                .split(" · ")
                .filter(|part| part.trim_start().starts_with('/'))
                .map(|part| part.trim().to_owned())
                .collect()
        })
        .unwrap_or_default();
    actions.extend(next_action(snapshot, task));
    if !actions.is_empty() {
        lines.extend(labeled(
            "Next",
            &actions.join(GAP),
            width,
            Style::default().fg(theme.color(Tone::Cyan)),
        ));
    }
    if let Some(name) =
        snapshot
            .receipts
            .iter()
            .rev()
            .find_map(|receipt| match &receipt.request.action {
                crate::tasks::Action::Branch { task: id, name, .. } if *id == task.id => Some(name),
                _ => None,
            })
    {
        lines.extend(labeled("Branch", name, width, normal));
    }
    let review = snapshot.review_summary_for_task(task.id);
    let criteria = snapshot.acceptance_for_task(task.id);
    if review.is_some() || !criteria.is_empty() {
        lines.push(Line::default());
    }
    if let Some(summary) = review {
        lines.extend(labeled("Review", &compact_review(&summary), width, normal));
    }
    for (index, item) in criteria.iter().enumerate() {
        // Numbered items hang under their text, not under the number.
        let number = format!("{}. ", index + 1);
        let room = width.saturating_sub(LABEL + number.width()).max(12);
        for (row, chunk) in wrap_words(&single_line(item), room).into_iter().enumerate() {
            let label = if index == 0 && row == 0 {
                "Criteria"
            } else {
                ""
            };
            let prefix = if row == 0 {
                number.clone()
            } else {
                " ".repeat(number.width())
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<LABEL$}"), dim()),
                Span::raw(prefix),
                Span::raw(chunk),
            ]));
        }
    }
    if live.is_none() {
        if task.status == TaskStatus::Running {
            let observation = tasks
                .run_observations
                .get(&task.id)
                .map(|text| safe(text))
                .unwrap_or_else(|| {
                    if tasks.workers.contains_key(&task.id) {
                        "Local worker registered · current activity not recorded".into()
                    } else {
                        "Current observation unavailable · recorded run does not prove a live worker"
                            .into()
                    }
                });
            lines.push(Line::default());
            lines.extend(labeled("Stage", &observation, width, amber));
        } else if let Some(run) = &task.run {
            lines.push(Line::default());
            match tasks.outcome(task).as_deref() {
                Some(Ok(outcome)) => {
                    let mut outcome = outcome.iter();
                    if let Some(first) = outcome.next() {
                        let text: String = first
                            .spans
                            .iter()
                            .map(|span| span.content.as_ref())
                            .collect();
                        let text = text
                            .trim_start_matches(['✓', '✗', '–', '‖', ' '])
                            .to_owned();
                        lines.extend(labeled("Result", &text, width, first.style));
                    }
                    lines.extend(outcome.cloned());
                }
                _ => lines.extend(labeled("Result", &single_line(&run.detail), width, normal)),
            }
            if run.evidence_sha256.is_some() {
                lines.push(Line::default());
                lines.extend(labeled("Evidence", "recorded", width, dim()));
            }
        }
    }
    if let Some(live) = live {
        // FILE blocks read as code: `▸ path` headings, marker lines hidden.
        let model_output = crate::worker::display_output(&live.model_output);
        for (label, output) in [
            ("Model output", &model_output),
            ("Check stdout", &live.stdout),
            ("Check stderr", &live.stderr),
        ] {
            if output.trim().is_empty() {
                continue;
            }
            lines.push(Line::default());
            lines.push(heading(label));
            let text = safe(output);
            let all: Vec<&str> = text.lines().collect();
            // The panel follows the tail; keep a bounded window of recent lines.
            lines.extend(
                all[all.len().saturating_sub(400)..]
                    .iter()
                    .map(|line| Line::from((*line).to_owned())),
            );
        }
        return (
            lines,
            format!(" Task #{} · {} ", task.id, task_state_word(tasks, task)),
            true,
            pinned,
        );
    }
    (
        lines,
        format!(" Task #{} · {} ", task.id, task_state_word(tasks, task)),
        false,
        0,
    )
}

/// Slice logical lines before Ratatui's u16 scroll limit. The full row offset is
/// local UI state; drawing and navigation never mutate canonical task records.
/// A live panel follows its tail until the user scrolls away from it.
pub(super) fn task_panel(
    frame: &mut Frame,
    area: Rect,
    tasks: &crate::task_control::TaskControl,
    lines: Vec<Line<'static>>,
    title: String,
    live: bool,
) {
    task_panel_pinned(frame, area, tasks, lines, title, live, 0);
}

/// `pinned` leading lines stay fixed above the scrolling part when there is room.
pub(super) fn task_panel_pinned(
    frame: &mut Frame,
    area: Rect,
    tasks: &crate::task_control::TaskControl,
    mut lines: Vec<Line<'static>>,
    title: String,
    live: bool,
    pinned: usize,
) {
    frame.render_widget(Clear, area);
    let block = frame_block(area, title);
    let mut inner = block.inner(area);
    frame.render_widget(block, area);
    let pinned = pinned.min(lines.len());
    if pinned > 0 && usize::from(inner.height) >= pinned * 2 + 2 {
        let head: Vec<_> = lines.drain(..pinned).collect();
        let rows = Paragraph::new(head.clone())
            .wrap(Wrap { trim: false })
            .line_count(inner.width)
            .min(usize::from(inner.height) / 2) as u16;
        let split = Layout::vertical([Constraint::Length(rows), Constraint::Min(1)]).split(inner);
        frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), split[0]);
        inner = split[1];
    }
    tasks.scroll_height.set(inner.height);
    tasks.detail_live.set(live);
    if inner.width == 0 || inner.height == 0 {
        tasks.scroll_max.set(0);
        return;
    }
    let heights: Vec<usize> = lines
        .iter()
        .map(|line| {
            if line.width() <= usize::from(inner.width) {
                1
            } else {
                Paragraph::new(line.clone())
                    .wrap(Wrap { trim: false })
                    .line_count(inner.width)
                    .max(1)
            }
        })
        .collect();
    let maximum = heights
        .iter()
        .sum::<usize>()
        .saturating_sub(usize::from(inner.height));
    tasks.scroll_max.set(maximum);
    let mut remaining = if live && tasks.follow_tail.get() {
        maximum
    } else {
        tasks.scroll.min(maximum)
    };
    let mut first = 0;
    for rows in heights {
        if remaining < rows {
            break;
        }
        remaining -= rows;
        first += 1;
    }
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(first).collect::<Vec<_>>())
            .wrap(Wrap { trim: false })
            .scroll((remaining.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
}

/// A stored response reference identifies a historical claim; the current
/// canonical journal must still prove it before the header acknowledges a receipt.
pub(super) fn response_heading(
    source: Option<&crate::model::ResponseSource>,
    scope: Option<&crate::understanding::Snapshot>,
    origin: Option<&crate::understanding::Request>,
) -> (String, Color) {
    use crate::{model::ResponseSource, understanding::Action};
    let Some(source) = source else {
        return ("Assistant · source unrecorded".into(), Color::Green);
    };
    if let ResponseSource::Wayfinder {
        receipt: Some(reference),
    } = source
    {
        let verified = reference
            .revision
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
            .and_then(|index| {
                scope
                    .filter(|scope| scope.revision >= reference.revision)?
                    .receipts
                    .get(index)
            })
            .is_some_and(|receipt| {
                let actor = match receipt.request.action {
                    Action::Enter { .. } => "wayfinder-alfredo",
                    Action::Draft { .. } | Action::Confirm { .. } => "mission-commander",
                };
                receipt.request.expected_revision.checked_add(1) == Some(reference.revision)
                    && receipt.request.correlation == reference.correlation
                    && receipt.actor == actor
                    && origin.is_none_or(|request| request == &receipt.request)
            });
        if !verified {
            return (
                format!(
                    "Wayfinder · saved receipt {} unverified",
                    reference.revision
                ),
                Color::Yellow,
            );
        }
    }
    (source.label(), Color::Green)
}

pub(super) fn verified_receipt<'a>(
    snapshot: Option<&'a crate::tasks::Snapshot>,
    revision: u64,
    task: u64,
    correlation: &str,
) -> Option<&'a crate::tasks::Receipt> {
    revision
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| snapshot?.receipts.get(index))
        .filter(|receipt| {
            receipt.revision == revision
                && receipt.task == task
                && receipt.request.correlation == correlation
        })
}

/// A compact projection of an exact canonical receipt, never model-authored text.
/// The receipt revision and correlation stay available in the F4 activity view.
pub(super) fn task_receipt_lines(
    reference: &crate::model::TaskReceiptRef,
    snapshot: Option<&crate::tasks::Snapshot>,
) -> [Line<'static>; 2] {
    let line = match verified_receipt(
        snapshot,
        reference.revision,
        reference.task,
        &reference.correlation,
    ) {
        Some(receipt) => {
            let (glyph, color) = receipt_style(receipt);
            Line::styled(
                format!("{glyph} {}", short_phase(receipt)),
                Style::default().fg(color),
            )
        }
        None => Line::styled(
            format!("? Task #{} update not verified", reference.task),
            Style::default().fg(Color::Yellow),
        ),
    };
    [line, Line::default()]
}

pub(super) fn receipt_style(receipt: &crate::tasks::Receipt) -> (&'static str, Color) {
    use crate::tasks::{Action, TaskStatus};
    match &receipt.request.action {
        Action::Finish {
            status: TaskStatus::Failed,
            ..
        } => ("✗", Color::Red),
        Action::Finish {
            status: TaskStatus::Cancelled,
            ..
        }
        | Action::Cancel { .. } => ("–", Color::Yellow),
        Action::Decide { decision, .. } if decision.risk.is_some() => ("‖", Color::Magenta),
        _ => ("✓", Color::Green),
    }
}
