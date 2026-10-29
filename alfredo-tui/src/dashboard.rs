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

/// Line breaks (NEL, line and paragraph separators): each starts a new line in a log or
/// prompt, so single-line output turns them into a space and [`hidden`] flags them.
const LINE_SEPARATORS: [char; 3] = ['\u{0085}', '\u{2028}', '\u{2029}'];

/// Controls, line separators ([`LINE_SEPARATORS`], issue #361) and format (Cf)
/// characters: bidi overrides and isolates, zero-width and invisible marks could make a
/// prompt or an approval card show other text than runs.
/// Shared by `propose_plan` and project notes (stripped) and the approval card (shown,
/// see [`safe_visible`]).
pub fn hidden(c: char) -> bool {
    c.is_control()
        || LINE_SEPARATORS.contains(&c)
        || matches!(
            c,
            '\u{00AD}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061C}'
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{08E2}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{110BD}'
                | '\u{110CD}'
                | '\u{13430}'..='\u{1343F}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0001}'
                | '\u{E0020}'..='\u{E007F}'
        )
}

/// Unicode Default_Ignorable_Code_Point (DerivedCoreProperties, Unicode 16) characters
/// that [`hidden`] does not already flag: they draw as nothing in most fonts and
/// renderers, so a card could show other text than the bytes hold (issue #468).
fn default_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{034F}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180D}'
            | '\u{180F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// What the approval card and the chat transcript show as `<U+XXXX>` instead of drawing:
/// [`hidden`] plus every Default_Ignorable code point. One list, so the card and the
/// transcript agree (issues #305, #436, #468). Narrower than widening [`hidden`], which
/// `propose_plan`, project notes and the planner also use, so their text stays as typed.
pub fn card_hidden(c: char) -> bool {
    hidden(c) || default_ignorable(c)
}

/// Like [`safe`], but a character that [`card_hidden`] flags is shown as `<U+XXXX>`
/// instead of drawn or dropped, so an approval card never shows other text than the bytes
/// hold (issues #305, #468). That covers every C0/C1 control except newline and tab, so a
/// bare CR (Python, Node and YAML read it as a line break) and an ESC never vanish.
/// Text, CJK, accents and emoji are unchanged.
pub fn safe_visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\n' || c == '\t' || !card_hidden(c) {
            out.push(c);
        } else {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        }
    }
    out
}

/// Tab stops are every `TAB_WIDTH` columns (issue #404).
pub const TAB_WIDTH: usize = 4;

/// Replace each tab with spaces up to the next [`TAB_WIDTH`] stop, counting columns per
/// line (a `\n` restarts at 0) and by display width, so tab-indented code keeps its
/// indent and `a\tb` keeps a gap. Display only: ratatui draws a raw tab as nothing.
pub fn expand_tabs(text: &str) -> String {
    tabs_as(text, false)
}

/// Like [`expand_tabs`], but each tab starts with `→` so a tab never looks like the
/// spaces next to it (the approval card, issue #404). A literal `→` in the text shows as
/// `<U+2192>`, so a file holding the arrow cannot pass for a tab.
pub fn show_tabs(text: &str) -> String {
    tabs_as(text, true)
}

fn tabs_as(text: &str, marked: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.chars() {
        match c {
            '\n' => {
                column = 0;
                out.push(c);
            }
            '\t' => {
                let pad = TAB_WIDTH - column % TAB_WIDTH;
                column += pad;
                if marked {
                    out.push('→');
                }
                out.extend(std::iter::repeat_n(' ', pad - usize::from(marked)));
            }
            '→' if marked => {
                out.push_str("<U+2192>");
                column += "<U+2192>".len();
            }
            _ => {
                column += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                out.push(c);
            }
        }
    }
    out
}

/// [`single_line_visible`] that keeps tabs as spaces to the next stop instead of
/// flattening each to one space (tool-result rows, issue #404).
pub fn single_line_visible_tabs(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| {
            if c == '\n' || LINE_SEPARATORS.contains(&c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    expand_tabs(&safe_visible(&flat))
}

/// [`safe_visible`] on one line.
///
/// Control characters are dropped as before (headless stderr, issue #361): only the
/// Default_Ignorable and format characters of [`card_hidden`] are shown as `<U+XXXX>`
/// (issue #468).
pub fn single_line_visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in line_breaks_as_spaces(text).chars() {
        if !card_hidden(c) {
            out.push(c);
        } else if !c.is_control() {
            out.push_str(&format!("<U+{:04X}>", c as u32));
        }
    }
    out
}

/// LF, tab and the [`LINE_SEPARATORS`] become a space, so single-line output never starts
/// a second line (issue #361).
fn line_breaks_as_spaces(text: &str) -> String {
    text.chars()
        .map(|c| {
            if matches!(c, '\n' | '\t') || LINE_SEPARATORS.contains(&c) {
                ' '
            } else {
                c
            }
        })
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
        // A pre-policy save can hold an approval with no policy; the run guard refuses it.
        TaskStatus::Approved if task.policy.is_none() => "needs policy",
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
    // No `0/0` while the plan is still being made.
    let mut fields = Vec::new();
    if status.total > 0 {
        fields.push(format!("{}/{}", status.done, status.total));
    }
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
    safe(&line_breaks_as_spaces(text))
}

/// Which area the footer describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Footer {
    /// The side pane has focus (F6).
    Pane,
    /// Task detail on the right.
    Tasks,
    /// The plan draft owns the right pane.
    Plan,
    /// The plan draft owns the right pane while a manual plan request generates: Esc cancels it.
    PlanRunning,
    /// An agent view owns the right pane and the prompt.
    Agent,
    /// Chat on the right.
    Chat,
    /// Chat on the right with every tool result shown in full (Ctrl+O, issue #258).
    ChatExpanded,
}

/// Where the chat turn stands; the chat footers name the key that applies (issue #399).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// No turn is running and none needs a retry.
    Idle,
    /// A reply is thinking, streaming or running a tool: Esc stops it.
    Running,
    /// The last turn was cancelled or failed: Ctrl+R retries it.
    Interrupted,
}

/// Footer hints for the right pane: task detail or chat.
pub fn footer_hints(dashboard: bool, width: usize) -> String {
    footer(
        if dashboard {
            Footer::Tasks
        } else {
            Footer::Chat
        },
        Turn::Idle,
        width,
    )
}

/// One line, at most eight hints separated by two spaces, relevant to the
/// focused area; lower-priority hints drop first when narrow.
pub fn footer(context: Footer, turn: Turn, width: usize) -> String {
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
        Footer::Plan => &[
            ("F6 pane", 1),
            ("^O raw/tasks", 1),
            ("F2 chat", 1),
            ("PgUp/Dn scroll", 2),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
        Footer::PlanRunning => &[
            ("Esc cancel plan", 0),
            ("F6 pane", 1),
            ("^O raw/tasks", 1),
            ("F2 chat", 1),
            ("PgUp/Dn scroll", 2),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
        Footer::Agent => &[
            ("Enter send", 1),
            ("Esc back", 1),
            ("^O expand", 2),
            ("F6 pane", 2),
            ("PgUp/Dn scroll", 3),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
        Footer::Chat | Footer::ChatExpanded => &[
            // Priority 0 keeps the turn hint at every width, like help and quit.
            (
                match turn {
                    Turn::Running => "Esc stop",
                    Turn::Interrupted => "Ctrl+R retries",
                    Turn::Idle => "",
                },
                0,
            ),
            ("Enter send", 1),
            ("F6 pane", 1),
            ("F2 tasks", 1),
            (
                if context == Footer::ChatExpanded {
                    "^O collapse results"
                } else {
                    "^O expand results"
                },
                2,
            ),
            ("^N new", 2),
            ("PgUp/Dn/Home/End scroll", 3),
            ("F1 help", 0),
            ("^Q quit", 0),
        ],
    };
    let mut level = 5;
    loop {
        let chosen: Vec<&str> = hints
            .iter()
            .filter(|(hint, priority)| *priority <= level && !hint.is_empty())
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

#[cfg(test)]
mod visible_tests {
    use super::*;

    #[test]
    fn format_characters_show_as_code_points_and_text_is_unchanged() {
        assert_eq!(
            safe_visible("a\u{202E}b\u{200B}c\u{E0041}\u{7}d"),
            "a<U+202E>b<U+200B>c<U+E0041><U+0007>d"
        );
        let plain = "你好 café 🙂\n\tx";
        assert_eq!(safe_visible(plain), plain);
        assert_eq!(single_line_visible("a\u{2066}\nb"), "a<U+2066> b");
        assert!(hidden('\u{FEFF}') && !hidden('é'));
    }

    /// Issue #468: every C0/C1 control except newline and tab shows as a code point instead
    /// of being dropped, so the card never shows other text than the bytes hold.
    #[test]
    fn every_c0_and_c1_control_shows_as_a_code_point() {
        assert_eq!(
            safe_visible("a\x01b\x1bc\x7fd\u{85}e\x07f\rg"),
            "a<U+0001>b<U+001B>c<U+007F>d<U+0085>e<U+0007>f<U+000D>g"
        );
        assert_eq!(safe_visible("a\n\tb"), "a\n\tb");
    }

    /// Issue #468: a lone CR shows as a code point; a Default_Ignorable character the
    /// planner and notes do not reject still shows on the card; emoji keep their text.
    #[test]
    fn a_lone_cr_and_default_ignorables_show_on_the_card() {
        assert_eq!(safe_visible("x  # n\rimport os"), "x  # n<U+000D>import os");
        assert_eq!(safe_visible("a\r\nb"), "a<U+000D>\nb");
        assert_eq!(
            safe_visible("sh\u{034F}./a\u{FE0F}b"),
            "sh<U+034F>./a<U+FE0F>b"
        );
        assert!(!hidden('\u{034F}') && card_hidden('\u{034F}'));
        assert_eq!(safe_visible("👨\u{200D}👩 ❤"), "👨<U+200D>👩 ❤");
    }

    /// Issue #404: tabs expand to the next 4-column stop per line; the card form marks
    /// each with `→` and escapes a literal arrow.
    #[test]
    fn tabs_expand_to_the_next_stop_and_the_card_marks_them() {
        assert_eq!(expand_tabs("\tif x {"), "    if x {");
        assert_eq!(expand_tabs("a\tb\tc"), "a   b   c");
        assert_eq!(expand_tabs("abcd\te"), "abcd    e");
        assert_eq!(expand_tabs("你\tb"), "你  b");
        assert_eq!(expand_tabs("a\n\tb"), "a\n    b");
        assert_eq!(show_tabs("\tx"), "→   x");
        assert_eq!(show_tabs("abc\tx"), "abc→x");
        assert_eq!(show_tabs("a\tb→"), "a→  b<U+2192>");
        assert_ne!(show_tabs("a\tb"), show_tabs("a   b"));
        assert_ne!(show_tabs("a\tb"), show_tabs("a→  b"));
        assert_eq!(
            single_line_visible_tabs("1\tx\u{2028}\t\u{202E}"),
            "1   x   <U+202E>"
        );
        assert!(!expand_tabs("a\tb").contains('\t') && !show_tabs("a\tb").contains('\t'));
    }

    /// Issue #361: line separators are line breaks, so every single-line helper turns them
    /// into a space, and the project-notes and plan strips (which use `hidden`) drop them.
    #[test]
    fn line_separators_never_reach_single_line_output_raw() {
        let text = "a\u{2028}b\u{2029}c\u{85}d";
        assert_eq!(single_line(text), "a b c d");
        assert_eq!(single_line_visible(text), "a b c d");
        assert_eq!(safe_visible("a\u{2028}b"), "a<U+2028>b");
        assert!(hidden('\u{2028}') && hidden('\u{2029}') && hidden('\u{85}'));
    }

    /// Issue #361: the bidi isolates, marks and Arabic letter mark stay covered by `hidden`.
    #[test]
    fn bidi_isolates_and_marks_are_hidden() {
        for c in ['\u{2066}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}'] {
            assert!(hidden(c), "{c:?}");
        }
    }
}
