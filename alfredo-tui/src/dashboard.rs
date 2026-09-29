//! Presentation helpers for the multi-agent dashboard. Read-only: nothing here
//! records state, authorizes work or changes canonical task receipts.
use crate::tasks::{Snapshot, Task, TaskStatus};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};
use unicode_width::UnicodeWidthStr;

pub fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// Truncate to `width` terminal cells, marking the cut with `…`.
pub fn truncate(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > width - 1 {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// One status glyph per task with its colour.
pub fn glyph(snapshot: &Snapshot, task: &Task) -> (&'static str, Color) {
    match task.status {
        TaskStatus::Accepted => ("✓", Color::Green),
        TaskStatus::Running => ("▶", Color::Yellow),
        TaskStatus::ReviewReady => ("◐", Color::Cyan),
        TaskStatus::NeedsHumanReview => ("‖", Color::Magenta),
        TaskStatus::Failed | TaskStatus::Rejected => ("✗", Color::Red),
        TaskStatus::Cancelled => ("–", Color::DarkGray),
        TaskStatus::Proposed | TaskStatus::Approved if blocked(snapshot, task) => {
            ("‖", Color::Magenta)
        }
        TaskStatus::Proposed | TaskStatus::Approved => ("○", Color::Gray),
    }
}

/// A dependency that ended without an accepted result (and no accepted repair)
/// holds this task; ordinary pending dependencies do not.
fn blocked(snapshot: &Snapshot, task: &Task) -> bool {
    task.dependencies.iter().any(|id| {
        snapshot.dependency_source(*id).is_err()
            && snapshot.tasks.iter().any(|parent| {
                parent.id == *id
                    && matches!(
                        parent.status,
                        TaskStatus::Failed
                            | TaskStatus::Rejected
                            | TaskStatus::Cancelled
                            | TaskStatus::NeedsHumanReview
                    )
            })
    })
}

/// Short human state word for titles.
pub fn state_word(task: &Task) -> &'static str {
    match task.status {
        TaskStatus::Proposed => "proposed",
        TaskStatus::Approved => "approved",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::Running => "running",
        TaskStatus::ReviewReady => "awaiting review",
        TaskStatus::NeedsHumanReview => "held for review",
        TaskStatus::Accepted => "accepted",
        TaskStatus::Failed => "failed",
        TaskStatus::Rejected => "rejected",
    }
}

/// Original tasks done and total, plus repair tasks counted separately. An
/// original task resolved by an accepted repair counts as done.
pub fn done_total(snapshot: &Snapshot) -> (usize, usize, usize) {
    let originals: Vec<&Task> = snapshot
        .tasks
        .iter()
        .filter(|task| task.repair_of.is_none())
        .collect();
    let done = originals
        .iter()
        .filter(|task| {
            task.status == TaskStatus::Accepted || snapshot.resolution_for_family(task.id).is_some()
        })
        .count();
    (
        done,
        originals.len(),
        snapshot.tasks.len() - originals.len(),
    )
}

pub fn clock(elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Autopilot header facts after its state: done/total, failures and repairs
/// only when present, elapsed, branch. The goal is the group title instead.
pub fn autopilot_fields(status: &crate::autopilot::Status) -> Vec<String> {
    let mut fields = vec![format!("{}/{}", status.done, status.total)];
    if status.failed > 0 {
        fields.push(format!("{} failed", status.failed));
    }
    if status.repairs > 0 {
        fields.push(format!(
            "{} repair{}",
            status.repairs,
            if status.repairs == 1 { "" } else { "s" }
        ));
    }
    fields.push(clock(status.elapsed));
    if let Some(branch) = &status.branch {
        fields.push(single_line(branch));
    }
    fields
}

/// Header row 2: `Autopilot ✓ done   1/1   01:02   alfredo/go-…`.
pub fn autopilot_row(status: &crate::autopilot::Status, width: usize) -> String {
    let mut row = format!(
        " Autopilot {} {}",
        status.state.marker(),
        status.state.label()
    );
    for field in autopilot_fields(status) {
        row.push_str("   ");
        row.push_str(&field);
    }
    truncate(&row, width)
}

pub fn single_line(text: &str) -> String {
    safe(text).replace(['\n', '\t'], " ")
}

/// Which area the footer describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Footer {
    /// The side pane has focus (F6).
    Pane,
    /// Task detail on the right.
    Tasks,
    /// Chat on the right.
    Chat,
}

/// Footer hints for the right pane: task detail or chat.
pub fn footer_hints(dashboard: bool, width: usize) -> String {
    footer(
        if dashboard {
            Footer::Tasks
        } else {
            Footer::Chat
        },
        width,
    )
}

/// One line, at most eight hints separated by two spaces, relevant to the
/// focused area; lower-priority hints drop first when narrow.
pub fn footer(context: Footer, width: usize) -> String {
    let hints: &[(&str, u8)] = match context {
        Footer::Pane => &[
            ("↑↓ move", 1),
            ("Tab section", 1),
            ("Enter open", 1),
            ("Esc prompt", 2),
            ("Alt+←/→ fold", 3),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
        Footer::Tasks => &[
            ("F6 pane", 1),
            ("↑↓ task", 1),
            ("F2 chat", 1),
            ("F3 evidence", 2),
            ("F5 pause", 3),
            ("PgUp/Dn scroll", 3),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
        Footer::Chat => &[
            ("Enter send", 1),
            ("F6 pane", 1),
            ("F2 tasks", 1),
            ("^N new", 2),
            ("Tab switch", 3),
            ("PgUp/Dn scroll", 3),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
    };
    let mut level = 5;
    loop {
        let chosen: Vec<&str> = hints
            .iter()
            .filter(|(_, priority)| *priority <= level)
            .map(|(hint, _)| *hint)
            .collect();
        // Keep the quit/help pair at the end so the line always closes on a whole hint.
        let (rest, tail): (Vec<&str>, Vec<&str>) = chosen
            .into_iter()
            .partition(|hint| !matches!(*hint, "F1 help" | "^Q quit"));
        let line = rest.into_iter().chain(tail).collect::<Vec<_>>().join("  ");
        if line.width() <= width || level == 0 {
            return truncate(&line, width);
        }
        level -= 1;
    }
}

/// Compact projection of verified run evidence: outcome first, then diff and output.
/// Run, baseline and receipt identifiers stay in the F3 evidence view.
pub fn outcome_lines(raw: &str) -> Result<Vec<Line<'static>>, String> {
    let evidence: crate::worker::Evidence =
        serde_json::from_str(raw).map_err(|_| "Cannot display malformed run evidence")?;
    let heading = |text: &str| {
        Line::styled(
            text.to_owned(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    };
    let failed_check = evidence
        .check
        .as_ref()
        .filter(|check| !crate::worker::check_passed(check));
    let tail = failed_check.and_then(crate::worker::output_tail);
    let mut detail = single_line(&evidence.detail);
    if failed_check.is_some() {
        // The compact tail in the detail is shown below as formatted lines instead.
        for marker in [": stderr: ", ": stdout: ", " · stderr: ", " · stdout: "] {
            if let Some(index) = detail.find(marker) {
                detail.truncate(index);
            }
        }
        detail = detail.trim_end().trim_end_matches(':').to_string();
    }
    let (text, color) = match evidence.status {
        TaskStatus::ReviewReady | TaskStatus::Accepted => {
            ("✓ Check passed".to_owned(), Color::Green)
        }
        TaskStatus::Cancelled => (format!("– Run cancelled · {detail}"), Color::Yellow),
        TaskStatus::NeedsHumanReview => (format!("‖ Held · {detail}"), Color::Magenta),
        _ => (format!("✗ Run failed · {detail}"), Color::Red),
    };
    let passed = color == Color::Green;
    let exit = evidence
        .check
        .as_ref()
        .and_then(|check| check.exit_code)
        .map(|code| format!(" · exit {code}"))
        .unwrap_or_default();
    // A pass is one line; a failure keeps the check status on its own line.
    let mut lines = vec![Line::styled(
        if passed {
            format!("{text}{exit}")
        } else {
            text
        },
        Style::default().fg(color),
    )];
    match &evidence.check {
        Some(_) if passed => {}
        // The outcome line already states the check result; do not repeat it.
        Some(check) if detail.starts_with("Check ") => {
            let message = single_line(&check.error_message);
            if !message.is_empty() && !detail.contains(&message) {
                lines.push(Line::styled(message, Style::default().fg(Color::Red)));
            }
        }
        Some(check) => {
            lines.push(Line::from(format!(
                "Check · {}{exit}",
                single_line(&check.status)
            )));
            if !check.error_message.is_empty() {
                lines.push(Line::styled(
                    single_line(&check.error_message),
                    Style::default().fg(Color::Red),
                ));
            }
        }
        None => lines.push(Line::from("Check · no check result retained")),
    }
    if let Some((label, tail)) = &tail {
        lines.push(Line::default());
        lines.push(heading(&format!("{label} · last lines")));
        lines.extend(
            tail.lines()
                .map(|line| Line::styled(line.to_owned(), Style::default().fg(Color::Red))),
        );
    }
    lines.push(Line::default());
    lines.push(heading("Diff"));
    if evidence.patch.is_empty() {
        lines.push(Line::from("No changes captured."));
    } else {
        for line in safe(&evidence.patch).lines() {
            let color = if line.starts_with("diff --git ")
                || line.starts_with("+++ ")
                || line.starts_with("--- ")
            {
                Color::Cyan
            } else if line.starts_with("@@") {
                Color::Yellow
            } else if line.starts_with('+') {
                Color::Green
            } else if line.starts_with('-') {
                Color::Red
            } else {
                Color::Reset
            };
            lines.push(Line::styled(line.to_owned(), Style::default().fg(color)));
        }
    }
    if let Some(check) = &evidence.check {
        for (label, output) in [
            ("Check stdout", &check.stdout),
            ("Check stderr", &check.stderr),
        ] {
            // A stream already shown whole as the tail above is not repeated.
            let shown = tail.as_ref().is_some_and(|(stream, text)| {
                label.ends_with(stream) && text.lines().count() >= output.trim_end().lines().count()
            });
            if !output.trim().is_empty() && !shown {
                lines.push(Line::default());
                lines.push(heading(label));
                lines.extend(safe(output).lines().map(|line| Line::from(line.to_owned())));
            }
        }
    }
    Ok(lines)
}

/// Live snapshot of a running worker for display.
pub struct Live {
    pub stage: String,
    pub cancelling: bool,
    pub model_output: String,
    pub stdout: String,
    pub stderr: String,
}

/// Compact byte count.
pub fn bytes(count: usize) -> String {
    if count >= 1024 * 1024 {
        format!("{:.1} MB", count as f64 / (1024.0 * 1024.0))
    } else if count >= 1024 {
        format!("{:.1} KB", count as f64 / 1024.0)
    } else {
        format!("{count} B")
    }
}
