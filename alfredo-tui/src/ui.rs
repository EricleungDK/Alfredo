use crate::dashboard::{self, single_line, truncate};
use crate::model::{App, Status};
use crate::side_pane::{self, MissionRow, Projection, RowKind, Section, WorkRow};
use crate::task_control::TaskControl;
use crate::theme::{ColorMode, Record, RowStatus, Theme, Tone};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame,
};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

mod activity_lines;
mod inspector;

pub use activity_lines::short_phase;
use activity_lines::{collapsed_lines, collapsed_step, command_lines, repeated_failure, Phase};
use inspector::{
    receipt_style, response_heading, task_panel, task_panel_pinned, task_receipt_lines,
    verified_receipt, work_inspector,
};

/// Below this width the side pane collapses to one summary row (F6 overlay).
pub const PANE_BREAKPOINT: u16 = 88;
/// Detail label column; `Criteria ` is the longest label.
const LABEL: usize = 9;
/// Tree indentation stops at group › task › repair; repairs of repairs stay
/// at the repair level (their titles name their parent).
const MAX_DEPTH: usize = 2;
/// Cells of an automatic command shown in the transcript.
const COMMAND_SUMMARY: usize = 160;

/// Most text rows the prompt box shows for a multi-line draft (issue #398).
const MAX_PROMPT_ROWS: usize = 6;

fn safe(text: &str) -> String {
    dashboard::safe(text)
}

/// Approval cards and tool-step rows: format characters show as `<U+XXXX>` (issue #305).
fn safe_visible(text: &str) -> String {
    dashboard::safe_visible(text)
}

/// Fits a one-line notice into `width` cells, cutting at a word boundary with an ellipsis.
fn fit_words(text: &str, width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if text.width() <= width {
        return text.to_string();
    }
    let mut used = 0;
    let mut end = 0;
    let mut boundary = None;
    for (index, ch) in text.char_indices() {
        let cells = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cells > width.saturating_sub(1) {
            break;
        }
        if ch == ' ' {
            boundary = Some(index);
        }
        used += cells;
        end = index + ch.len_utf8();
    }
    // A word boundary that would discard most of the line (a long unbroken token such
    // as a JSON template) loses to a plain cut, so the visible start stays copyable.
    let cut = boundary.filter(|index| *index >= width / 2).unwrap_or(end);
    format!("{}…", text[..cut].trim_end())
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// The argument part of a `⚙` row (issue #274): a registered tool's summary (path or
/// primary argument first, long text cut), else every value in map order. The caller
/// still passes the row through `single_line_visible`.
fn tool_row_arguments(name: &str, arguments: &serde_json::Value) -> String {
    let normalized = crate::chat_tools::normalize_arguments(arguments);
    if normalized.is_object() {
        if let Some(summary) = crate::chat_tools::summary::summarize(name, &normalized) {
            return summary;
        }
    }
    match arguments {
        serde_json::Value::Object(map) => map
            .values()
            .map(|value| match value {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Transcript rows for one tool round: `⚙ name args` per call and the first line of
/// its result, cut to one row (issue #233). With `expanded` (Ctrl+O, issue #258) the
/// result follows in full, capped at `EXPANDED_RESULT_LINES` rows and a `… N more lines`
/// marker; the model always receives the full text either way.
/// `rule_of` names the always-allow rule that approved the call at that position of the
/// step (issue #320); the row then says `rule #N`.
/// Rows of one tool result shown when results are expanded.
const EXPANDED_RESULT_LINES: usize = 200;

fn tool_step_lines(
    step: &crate::model::ToolStep,
    width: usize,
    expanded: bool,
    rule_of: impl Fn(usize) -> Option<u64>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (position, (call, result)) in step.calls.iter().zip(&step.results).enumerate() {
        let rule = rule_of(position).map_or(String::new(), |id| format!(" (rule #{id})"));
        let arguments = tool_row_arguments(&call.name, &call.arguments);
        let call_text =
            dashboard::single_line_visible(&format!("⚙ {}{rule} {arguments}", call.name));
        lines.push(Line::styled(truncate(call_text.trim_end(), width), dim()));
        let count = result.lines().count();
        if expanded {
            lines.extend(result.lines().take(EXPANDED_RESULT_LINES).map(|line| {
                Line::styled(
                    format!("  ↳ {}", dashboard::single_line_visible_tabs(line)),
                    dim(),
                )
            }));
            if count > EXPANDED_RESULT_LINES {
                lines.push(Line::styled(
                    format!("  … {} more lines", count - EXPANDED_RESULT_LINES),
                    dim(),
                ));
            }
            continue;
        }
        let first = dashboard::single_line_visible_tabs(result.lines().next().unwrap_or(""));
        let more = if count > 1 {
            format!(" … ({count} lines)")
        } else {
            String::new()
        };
        let room = width.saturating_sub(4 + more.width());
        let first = if first.width() > room {
            truncate(&first, room)
        } else {
            first
        };
        lines.push(Line::styled(format!("  ↳ {first}{more}"), dim()));
    }
    lines.push(Line::default());
    lines
}

/// Side pane width: a quarter of the terminal, 28 to 44 columns.
pub fn pane_width(width: u16) -> u16 {
    (width / 4).clamp(28, 44)
}

/// A bordered block with one column of padding, only when the area can hold a complete box.
fn frame_block(area: Rect, title: String) -> Block<'static> {
    if area.height < 3 || area.width < 4 {
        Block::default()
    } else {
        Block::bordered()
            .title(title)
            .padding(Padding::horizontal(1))
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    draw_inner(frame, app, None);
}

pub fn draw_with_tasks(frame: &mut Frame, app: &App, tasks: &TaskControl) {
    draw_inner(frame, app, Some(tasks));
}

fn draw_inner(frame: &mut Frame, app: &App, tasks: Option<&TaskControl>) {
    let area = frame.area();
    if area.width < 32 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("Alfredo\nResize to at least 32 × 10\nCtrl+Q quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let now = Instant::now();
    let theme = app.pane.theme;
    let projection = side_pane::project(app, tasks, now);
    let autopilot = tasks.and_then(|tasks| tasks.autopilot.as_ref());
    // The prompt box grows with a multi-line draft; the transcript gives up the rows.
    let header_rows = 1 + u16::from(autopilot.is_some());
    let prompt_text_rows = (app.sessions[app.selected].draft_line_count())
        .min(MAX_PROMPT_ROWS)
        .min(usize::from(
            area.height.saturating_sub(header_rows + 3 + 2 + 2),
        ))
        .max(1);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(autopilot.is_some())),
        Constraint::Min(3),
        Constraint::Length(prompt_text_rows as u16 + 2),
        Constraint::Length(2),
    ])
    .split(area);
    let session = &app.sessions[app.selected];
    draw_header(frame, app, tasks, rows[0]);
    if let Some(status) = autopilot {
        draw_autopilot(frame, &theme, status, rows[1]);
    }
    let body = rows[2];
    let wide = area.width >= PANE_BREAKPOINT;
    let right = if wide {
        let split = Layout::horizontal([
            Constraint::Length(pane_width(area.width)),
            Constraint::Min(1),
        ])
        .split(body);
        draw_side_pane(frame, split[0], app, &projection, now);
        split[1]
    } else if body.height >= 6 {
        let split = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(body);
        draw_summary(frame, split[0], app, &projection);
        split[1]
    } else {
        // Tiny terminals keep every row for the detail; F6 still opens the pane.
        body
    };
    let help = app.completion.as_ref().filter(|completion| completion.help);
    if let Some(help) = help {
        draw_help(frame, help, right);
    } else {
        draw_right(frame, app, tasks, right, now);
    }
    if !wide && (app.pane.overlay || app.pane.focus.is_some()) {
        draw_side_pane(frame, body, app, &projection, now);
    }
    let focused = app.pane.focus.is_some();
    let agent = tasks.and_then(|tasks| Some((tasks, tasks.agent_shown()?.target)));
    let prompt_title = match agent {
        _ if focused => " Prompt · Esc returns ".to_string(),
        _ if session.pending_approval().is_some() && !session.draft.is_empty() => {
            " Approval pending · clear the draft to answer y/n/a · Esc denies ".to_string()
        }
        _ if session
            .pending_approval()
            .is_some_and(crate::chat_tools::allow::available) =>
        {
            " Approval pending · y approve · a always · n deny · Esc cancels the turn ".to_string()
        }
        _ if session.pending_approval().is_some() => {
            " Approval pending · y approve · n deny · Esc cancels the turn ".to_string()
        }
        Some((tasks, target)) => crate::agent_view::prompt_title(tasks, target),
        None => " Prompt · Enter send ".to_string(),
    };
    let draft_width = rows[3].width.saturating_sub(4) as usize;
    let draft = if session.draft_line_count() > 1 {
        Text::from(
            session
                .draft_lines(draft_width, prompt_text_rows)
                .iter()
                .map(|line| Line::from(safe(line)))
                .collect::<Vec<_>>(),
        )
    } else {
        Text::from(safe(&session.draft_view(draft_width)))
    };
    frame.render_widget(
        Paragraph::new(draft)
            .style(if session.replace_pending() {
                // Rejected text reads as selected: the next character replaces it.
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            })
            .block(
                Block::bordered()
                    .padding(Padding::horizontal(1))
                    .border_style(Style::default().fg(if focused {
                        theme.color(Tone::Dim)
                    } else {
                        theme.color(Tone::Cyan)
                    }))
                    .title(prompt_title),
            ),
        rows[3],
    );
    let note = if !app.notice.is_empty() {
        safe(&app.notice)
    } else if let Some(tasks) = tasks.filter(|tasks| tasks.visible) {
        safe(&tasks.notice)
    } else if let Status::Failed(error) = &session.status {
        safe(error)
    } else {
        String::new()
    };
    let context = if focused {
        dashboard::Footer::Pane
    } else if agent.is_some() {
        dashboard::Footer::Agent
    } else if tasks.is_some_and(|tasks| tasks.visible && tasks.planner.visible) {
        dashboard::Footer::Plan
    } else if tasks.is_some_and(|tasks| tasks.visible) {
        dashboard::Footer::Tasks
    } else if session.tool_results_expanded {
        dashboard::Footer::ChatExpanded
    } else {
        dashboard::Footer::Chat
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                dashboard::footer(context, usize::from(area.width)),
                Style::default().fg(theme.color(Tone::Dim)),
            ),
            Line::styled(
                fit_words(&single_line(&note), usize::from(area.width)),
                Style::default().fg(theme.color(Tone::Amber)),
            ),
        ]),
        rows[4],
    );
    if theme.color == ColorMode::None {
        strip_colors(frame.buffer_mut());
    }
}

/// NO_COLOR: keep text and modifiers, drop every foreground and background colour.
fn strip_colors(buffer: &mut Buffer) {
    for cell in buffer.content.iter_mut() {
        cell.set_fg(Color::Reset);
        cell.set_bg(Color::Reset);
    }
}

/// Fields separated by three spaces, never chains of ` · `.
const GAP: &str = "   ";

/// Row 1: mission and repository name, attention items only when non-zero,
/// server health right-aligned. The full path stays in F4 and `/workspace`.
fn draw_header(frame: &mut Frame, app: &App, tasks: Option<&TaskControl>, area: Rect) {
    let theme = app.pane.theme;
    let health = health_span(app, area.width);
    let health_width = u16::try_from(health.width())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let split =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(health_width)]).split(area);
    frame.render_widget(Paragraph::new(Line::from(health)), split[1]);
    let mut badge = Style::default().add_modifier(Modifier::BOLD);
    badge = if theme.color == ColorMode::None {
        badge.add_modifier(Modifier::REVERSED)
    } else {
        badge.fg(Color::Black).bg(theme.color(Tone::Cyan))
    };
    let mut spans = vec![Span::styled(" ALFREDO ", badge), Span::raw(" ")];
    let mut room = usize::from(split[0].width).saturating_sub(11);
    let amber = Style::default().fg(theme.color(Tone::Amber));
    let mut attention = Vec::new();
    if let Some(tasks) = tasks {
        let status = tasks.work_status();
        if status.workers > 0 {
            attention.push(format!("{} running", status.workers));
        }
        if status.review > 0 {
            attention.push(format!("{} review", status.review));
        }
        let decisions = status.held + status.architect;
        if decisions > 0 {
            attention.push(format!(
                "{decisions} decision{}",
                if decisions == 1 { "" } else { "s" }
            ));
        }
        if tasks.dispatch.enabled {
            attention.push("dispatch on".into());
        }
        if tasks.scope_status.label.starts_with("Wayfinder /") {
            // `Wayfinder / MODE`; the full scope state is in /scope.
            let label = single_line(&tasks.scope_status.label);
            attention.push(label.split(" · ").next().unwrap_or_default().to_owned());
        }
    }
    let attention_width: usize = attention.iter().map(|item| GAP.len() + item.width()).sum();
    if let Some(snapshot) = tasks.and_then(|tasks| tasks.snapshot.as_ref()) {
        let repository = snapshot
            .workspace
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| snapshot.workspace.display().to_string());
        let mission = truncate(&single_line(&snapshot.mission), room);
        room = room.saturating_sub(mission.width());
        spans.push(Span::styled(
            mission,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        // Attention items outrank the repository name when space is short.
        let repository_room = room.saturating_sub(attention_width);
        if repository_room > 4 {
            let repository = truncate(&format!(" · {}", single_line(&repository)), repository_room);
            room = room.saturating_sub(repository.width());
            spans.push(Span::styled(
                repository,
                Style::default().fg(theme.color(Tone::Dim)),
            ));
        }
    }
    for item in attention {
        let field = format!("{GAP}{item}");
        if field.width() > room {
            break;
        }
        room -= field.width();
        spans.push(Span::styled(field, amber));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), split[0]);
}

fn health_span(app: &App, width: u16) -> Span<'static> {
    let session = &app.sessions[app.selected];
    let health = app.health.state(&session.model);
    let theme = app.pane.theme;
    health
        .label(side_pane::short_model(&session.model), width >= 100)
        .map(|label| {
            let tone = if !health.healthy() {
                Tone::Red
            } else if health.needs_attention() {
                Tone::Amber
            } else {
                Tone::Green
            };
            Span::styled(format!(" {label} "), Style::default().fg(theme.color(tone)))
        })
        .unwrap_or_default()
}

/// Row 2, only while an autopilot run exists. The goal is the group title in the tree.
fn draw_autopilot(frame: &mut Frame, theme: &Theme, status: &crate::autopilot::Status, area: Rect) {
    use crate::autopilot::RunState;
    let tone = match status.state {
        RunState::Done => Tone::Green,
        RunState::Partial | RunState::Paused => Tone::Amber,
        RunState::Failed => Tone::Red,
        _ => Tone::Lime,
    };
    let mut spans = vec![
        Span::styled(" Autopilot ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("{} {}", status.state.marker(), status.state.label()),
            Style::default().fg(theme.color(tone)),
        ),
    ];
    for (index, field) in dashboard::autopilot_fields(status).into_iter().enumerate() {
        let style = if index == 0 {
            Style::default()
        } else {
            Style::default().fg(theme.color(Tone::Dim))
        };
        spans.push(Span::styled(format!("{GAP}{field}"), style));
    }
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}

/// Narrow terminals: one summary row in place of the side pane.
fn draw_summary(frame: &mut Frame, area: Rect, app: &App, projection: &Projection) {
    let theme = app.pane.theme;
    let mut fields = Vec::new();
    if let Some(mission) = projection.missions.first() {
        fields.push(mission.name.clone());
    }
    let summary = &projection.summary;
    if summary.total > 0 {
        fields.push(format!("{}/{} done", summary.done, summary.total));
    }
    if summary.working > 0 {
        fields.push(format!("{} working", summary.working));
    }
    let chats = app
        .sessions
        .iter()
        .filter(|session| session.status.active())
        .count();
    if chats > 0 {
        fields.push(format!(
            "{chats} chat{} active",
            if chats == 1 { "" } else { "s" }
        ));
    }
    let hint = " F6 pane ";
    let text = truncate(
        &format!(" {}", fields.join(GAP)),
        usize::from(area.width).saturating_sub(hint.width() + 1),
    );
    let gap = usize::from(area.width).saturating_sub(text.width() + hint.width());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(text),
            Span::raw(" ".repeat(gap)),
            Span::styled(hint, Style::default().fg(theme.color(Tone::Dim))),
        ])),
        area,
    );
}

fn work_title(projection: &Projection) -> String {
    let summary = &projection.summary;
    if summary.total > 0 {
        format!(" work  {}/{} done ", summary.done, summary.total)
    } else {
        " work ".into()
    }
}

/// Missions above work, one bordered pane with a `├ work ┤` separator.
fn draw_side_pane(frame: &mut Frame, area: Rect, app: &App, projection: &Projection, now: Instant) {
    let pane = &app.pane;
    let theme = pane.theme;
    frame.render_widget(Clear, area);
    if area.width < 8 || area.height < 3 {
        return;
    }
    let title_style = |section: Section| {
        if pane.focus == Some(section) {
            Style::default()
                .fg(theme.color(Tone::Cyan))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.color(Tone::Dim))
        }
    };
    let inner = Block::bordered().inner(area);
    let missions = if projection.missions.is_empty() {
        0
    } else {
        let count = projection
            .missions
            .len()
            .min(usize::from(inner.height / 3).max(1));
        if usize::from(inner.height) >= count + 3 {
            count
        } else {
            0
        }
    };
    let title = if missions > 0 {
        Span::styled(" missions ", title_style(Section::Missions))
    } else {
        Span::styled(work_title(projection), title_style(Section::Work))
    };
    frame.render_widget(Block::bordered().title(Line::from(title)), area);
    let width = usize::from(inner.width);
    let mut work = inner;
    if missions > 0 {
        let cursor = pane.mission_index(projection).unwrap_or(0);
        let offset = if pane.focus == Some(Section::Missions) {
            cursor.saturating_sub(missions - 1)
        } else {
            0
        };
        let lines: Vec<Line> = projection
            .missions
            .iter()
            .enumerate()
            .skip(offset)
            .take(missions)
            .map(|(index, row)| {
                mission_line(
                    row,
                    width,
                    &theme,
                    pane.focus == Some(Section::Missions) && index == cursor,
                )
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                height: missions as u16,
                ..inner
            },
        );
        let y = inner.y + missions as u16 + 1;
        let buffer = frame.buffer_mut();
        buffer[(area.x, y)].set_symbol("├");
        buffer[(area.right() - 1, y)].set_symbol("┤");
        for x in area.x + 1..area.right() - 1 {
            buffer[(x, y)].set_symbol("─");
        }
        buffer.set_stringn(
            area.x + 1,
            y,
            work_title(projection),
            usize::from(area.width.saturating_sub(2)),
            title_style(Section::Work),
        );
        work = Rect {
            y: y + 1,
            height: inner.bottom().saturating_sub(y + 1),
            ..inner
        };
    }
    if work.height == 0 {
        return;
    }
    let focused = pane.focus == Some(Section::Work);
    let cursor = focused.then(|| pane.work_index(projection)).flatten();
    let mut lines = Vec::new();
    let mut selected = None;
    for (index, row) in projection.work.iter().enumerate() {
        let start = lines.len();
        let highlighted = cursor == Some(index);
        lines.push(work_line(row, width, &theme, now, highlighted));
        if let Some(second) = &row.second {
            let indent = 2 * row.depth.min(MAX_DEPTH) + 4;
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(indent + 1)),
                Span::styled(
                    fit_second(second, width.saturating_sub(indent + 2)),
                    Style::default().fg(theme.color(Tone::Dim)),
                ),
            ]));
        }
        if highlighted || (cursor.is_none() && row.current) {
            selected = Some(lines.len());
        }
        let _ = start;
    }
    let height = usize::from(work.height);
    let offset = selected.map_or(0, |end| end.saturating_sub(height));
    frame.render_widget(
        Paragraph::new(lines).scroll((offset.min(usize::from(u16::MAX)) as u16, 0)),
        work,
    );
}

fn mission_line(row: &MissionRow, width: usize, theme: &Theme, highlighted: bool) -> Line<'static> {
    let ascii = theme.icons == crate::theme::IconSet::Ascii;
    let marker = match (row.current, ascii) {
        (true, false) => "●",
        (false, false) => "·",
        (true, true) => "*",
        (false, true) => "-",
    };
    let right = row.progress.clone();
    let room = width.saturating_sub(4 + right.width() + 2);
    let name = truncate(&row.name, room);
    let gap = width
        .saturating_sub(4 + name.width() + right.width())
        .max(1);
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            marker,
            Style::default().fg(theme.color(if row.current { Tone::Cyan } else { Tone::Dim })),
        ),
        Span::raw(" "),
        Span::styled(
            name,
            if row.current {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ),
        Span::raw(" ".repeat(gap)),
        Span::styled(right, Style::default().fg(theme.color(Tone::Dim))),
        Span::raw(" "),
    ];
    if highlighted {
        spans[0] = Span::raw("›");
        for span in &mut spans {
            span.style = span.style.add_modifier(Modifier::REVERSED);
        }
    }
    Line::from(spans)
}

fn work_line(
    row: &WorkRow,
    width: usize,
    theme: &Theme,
    now: Instant,
    highlighted: bool,
) -> Line<'static> {
    let indent = "  ".repeat(row.depth.min(MAX_DEPTH));
    let content = width.saturating_sub(2);
    let mut spans = vec![Span::raw(" "), Span::raw(indent.clone())];
    let head = match row.kind {
        RowKind::Group => {
            let fold = theme.icons.fold(row.expanded);
            spans.push(Span::styled(
                format!("{fold} "),
                Style::default().fg(theme.color(Tone::Dim)),
            ));
            indent.width() + 2
        }
        RowKind::Record(record) => {
            let icon = theme.icons.record(record);
            let status = row.status.unwrap_or(RowStatus::Idle);
            spans.push(Span::styled(
                format!("{icon} "),
                Style::default().fg(theme.color(if record == Record::Agent {
                    Tone::Blue
                } else {
                    Tone::Dim
                })),
            ));
            spans.push(Span::styled(
                format!("{} ", theme.status_glyph(status, now)),
                Style::default().fg(theme.color(Theme::tone(status))),
            ));
            indent.width() + 4
        }
    };
    let right = if row.right.is_empty() {
        String::new()
    } else {
        format!(" {}", row.right)
    };
    let label = truncate(&row.label, content.saturating_sub(head + right.width()));
    let gap = content.saturating_sub(head + label.width() + right.width());
    let mut label_style = Style::default();
    if row.current || row.kind == RowKind::Group {
        label_style = label_style.add_modifier(Modifier::BOLD);
    }
    spans.push(Span::styled(label, label_style));
    spans.push(Span::raw(" ".repeat(gap)));
    spans.push(Span::styled(
        right,
        Style::default().fg(theme.color(Tone::Dim)),
    ));
    spans.push(Span::raw(" "));
    if highlighted {
        spans[0] = Span::raw("›");
        for span in &mut spans {
            span.style = span.style.add_modifier(Modifier::REVERSED);
        }
    }
    Line::from(spans)
}

/// `stage  model  elapsed`, shortening the model before dropping it.
fn fit_second(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.into();
    }
    if let [model, time] = text.split("  ").collect::<Vec<_>>()[..] {
        let short = side_pane::short_model(model);
        let room = width.saturating_sub(time.width() + 2);
        if room >= 4 {
            return format!("{}  {time}", truncate(short, room));
        }
        return truncate(time, width);
    }
    if let [stage, model, time] = text.split("  ").collect::<Vec<_>>()[..] {
        let short = side_pane::short_model(model);
        let candidate = format!("{stage}  {short}  {time}");
        if candidate.width() <= width {
            return candidate;
        }
        let room = width.saturating_sub(stage.width() + time.width() + 4);
        if room >= 4 {
            return format!("{stage}  {}  {time}", truncate(short, room));
        }
        let candidate = format!("{stage}  {time}");
        if candidate.width() <= width {
            return candidate;
        }
    }
    truncate(text, width)
}

/// F1: grouped command catalog, scrolling with the selection.
fn draw_help(frame: &mut Frame, help: &crate::commands::Completion, area: Rect) {
    let width = usize::from(area.width.saturating_sub(6));
    let mut items = Vec::new();
    let mut selected = 0;
    let mut group = None;
    for (index, choice) in help.choices.iter().enumerate() {
        if choice.group != group {
            group = choice.group;
            items.push(ListItem::new(Line::styled(
                choice.group.unwrap_or_default().to_owned(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        if index == help.selected {
            selected = items.len();
        }
        items.push(ListItem::new(truncate(
            &format!("  {:<16} {}", choice.name, choice.description),
            width,
        )));
    }
    frame.render_widget(Clear, area);
    let block = if area.height < 3 {
        Block::default()
    } else {
        Block::bordered()
            .padding(Padding::horizontal(1))
            .title(if help.query.is_empty() {
                " Help · type to filter · ↑↓ choose · Enter fills prompt · Esc close ".to_string()
            } else {
                format!(
                    " Help /{} · ↑↓ choose · Enter fills prompt · Esc close ",
                    help.query.trim_start_matches(['/', '@'])
                )
            })
            .title_bottom(
                " F2 tasks/chat  F3 evidence  F4 activity  F5 pause  F6 pane  Enter agent  ^O expand/raw plan ",
            )
    };
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_symbol("› ")
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
        &mut ListState::default().with_selected(Some(selected)),
    );
}

/// The right pane: chat, task detail or one of the focused task views.
fn draw_right(frame: &mut Frame, app: &App, tasks: Option<&TaskControl>, area: Rect, now: Instant) {
    let session = &app.sessions[app.selected];
    if tasks.is_none_or(|tasks| !tasks.visible) && !app.models_visible && app.completion.is_none() {
        draw_transcript(frame, app, tasks, area);
    }
    let width = usize::from(area.width.saturating_sub(4));
    if let Some(tasks) =
        tasks.filter(|tasks| tasks.visible && !app.models_visible && app.completion.is_none())
    {
        tasks.detail_live.set(false);
        if tasks.planner.visible {
            task_panel(
                frame,
                area,
                tasks,
                safe(&tasks.planner.view())
                    .lines()
                    .map(|s| Line::from(s.to_owned()))
                    .collect(),
                " Plan draft · /plan-revise · /plan-save · /plan-cancel ".into(),
                // Follow the streaming draft while it generates.
                tasks.planner.active(),
            );
        } else if let Some(evidence) = &tasks.evidence {
            let block = frame_block(
                area,
                format!(
                    " Verified run evidence · task #{} · /tasks to return ",
                    evidence.task
                ),
            );
            let inner = block.inner(area);
            frame.render_widget(Clear, area);
            frame.render_widget(block, area);
            let page = evidence.page(inner.width, inner.height, tasks.scroll);
            tasks.scroll_max.set(page.maximum);
            tasks.scroll_height.set(inner.height);
            frame.render_widget(
                Paragraph::new(page.lines)
                    .wrap(Wrap { trim: false })
                    .scroll((page.row, 0)),
                inner,
            );
        } else if let Some(query) = &tasks.activity {
            frame.render_widget(Clear, area);
            let title = format!(" Saved task activity · {} ", safe(query));
            if let Some(snapshot) = &tasks.snapshot {
                let block = frame_block(area, title);
                let inner = block.inner(area);
                frame.render_widget(block, area);
                tasks.scroll_height.set(inner.height);
                tasks.detail_live.set(false);
                let window = tasks.activity_window(snapshot, query, inner.width, inner.height);
                tasks.scroll_max.set(window.maximum);
                if inner.width > 0 && inner.height > 0 {
                    frame.render_widget(
                        Paragraph::new(window.lines)
                            .wrap(Wrap { trim: false })
                            .scroll((window.row, 0)),
                        inner,
                    );
                }
            } else {
                task_panel(
                    frame,
                    area,
                    tasks,
                    vec![Line::from("Waiting for acknowledged task state")],
                    title,
                    false,
                );
            }
        } else if let Some(report) = &tasks.autopilot_report {
            task_panel(
                frame,
                area,
                tasks,
                report_lines(report, width, &app.pane.theme),
                " Autopilot ".into(),
                false,
            );
        } else if let Some(scope) = &tasks.scope_view {
            task_panel(
                frame,
                area,
                tasks,
                safe(&scope.text())
                    .lines()
                    .map(|s| Line::from(s.to_owned()))
                    .collect(),
                " Shared Understanding · project scope ".into(),
                false,
            );
        } else if let Some(view) = &tasks.agent {
            draw_agent(frame, area, tasks, view, &app.pane.theme);
        } else {
            let tree = tasks.work_tree();
            let focused = tasks
                .focused_work_node()
                .and_then(|id| tree.rows.iter().find(|row| row.id == id));
            let (lines, title, live, pinned) =
                work_inspector(tasks, &tree, focused, width, &app.pane.theme, now);
            task_panel_pinned(frame, area, tasks, lines, title, live, pinned);
        }
    }

    if app.models_visible {
        frame.render_widget(Clear, area);
        let mut lines = vec![
            Line::from(safe(&app.models_notice)),
            Line::from("Workers: /assign ID MODEL · fresh approval required"),
            Line::default(),
        ];
        let header = lines.len();
        lines.extend(app.models.iter().enumerate().map(|(row, name)| {
            let text = format!(
                "{} {} {}",
                if row == app.models_cursor { "▸" } else { " " },
                if crate::models::same_model(name, &session.model) {
                    "›"
                } else {
                    " "
                },
                safe(name)
            );
            if row == app.models_cursor {
                Line::styled(
                    text,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Line::from(text)
            }
        }));
        // PageUp/PageDown scroll freely; the cursor row is always kept in view.
        let cursor = (header + app.models_cursor) as u16;
        let inner = area.height.saturating_sub(2).max(1);
        let scroll = if app.models.is_empty() {
            app.models_scroll
        } else {
            app.models_scroll
                .min(cursor)
                .max((cursor + 1).saturating_sub(inner))
        };
        frame.render_widget(
            Paragraph::new(lines).scroll((scroll, 0)).block(frame_block(
                area,
                " Models · ↑↓ choose · Enter select · Esc close ".into(),
            )),
            area,
        );
    }
    if let Some(completion) = &app.completion {
        let items: Vec<_> = completion
            .choices
            .iter()
            .map(|choice| ListItem::new(format!("{}  {}", choice.name, choice.description)))
            .collect();
        frame.render_widget(Clear, area);
        frame.render_stateful_widget(
            List::new(items)
                .block(frame_block(
                    area,
                    if completion.live {
                        " Commands · ↑↓ choose · Enter/Tab fills draft · Esc close ".into()
                    } else {
                        " Complete · ↑↓ choose · Enter fills draft · Esc close ".into()
                    },
                ))
                .highlight_symbol("› ")
                .highlight_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            area,
            &mut ListState::default().with_selected(Some(completion.selected)),
        );
    }
}

/// The autopilot report in the side pane language: coloured state line, the
/// goal on one line, dim labels, glyph-coloured task lines, dim task details.
fn report_lines(report: &str, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    const LABELS: [&str; 7] = [
        "Tasks", "Repairs", "Branch", "Reason", "Review", "Merge", "Elapsed",
    ];
    let glyph_tone = |text: &str| match text.chars().next() {
        Some('✓') => Some(Tone::Green),
        Some('✗') => Some(Tone::Red),
        Some('◐' | '‖') => Some(Tone::Amber),
        Some('▶' | '◌') => Some(Tone::Lime),
        _ => None,
    };
    let text = safe(report);
    let structured = text
        .lines()
        .next()
        .is_some_and(|first| glyph_tone(first).is_some() && first.contains("Autopilot"));
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if !structured {
            lines.push(Line::from(line.to_owned()));
            continue;
        }
        let label = LABELS.iter().chain(["Note"].iter()).find(|label| {
            line.strip_prefix(**label)
                .is_some_and(|rest| rest.starts_with("  "))
        });
        if index == 0 {
            let tone = glyph_tone(line).unwrap_or(Tone::Dim);
            lines.push(Line::styled(
                line.to_owned(),
                Style::default()
                    .fg(theme.color(tone))
                    .add_modifier(Modifier::BOLD),
            ));
        } else if index == 1 {
            lines.push(Line::from(truncate(line, width)));
        } else if let Some(label) = label {
            // A ten-column label: `Repairs   0`, values hang under themselves.
            let value = line[label.len()..].trim_start();
            for (row, chunk) in wrap_words(value, width.saturating_sub(10).max(12))
                .into_iter()
                .enumerate()
            {
                let head = if row == 0 {
                    format!("{label:<10}")
                } else {
                    " ".repeat(10)
                };
                lines.push(Line::from(vec![
                    Span::styled(head, Style::default().fg(theme.color(Tone::Dim))),
                    Span::raw(chunk),
                ]));
            }
        } else if let Some(tone) = glyph_tone(line) {
            let (glyph, rest) = line.split_at(line.chars().next().unwrap().len_utf8());
            lines.push(Line::from(vec![
                Span::styled(glyph.to_owned(), Style::default().fg(theme.color(tone))),
                Span::raw(truncate(rest, width.saturating_sub(1))),
            ]));
        } else if line.starts_with("  ") {
            lines.push(Line::styled(
                truncate(line, width),
                Style::default().fg(theme.color(Tone::Dim)),
            ));
        } else {
            lines.push(Line::from(line.to_owned()));
        }
    }
    lines
}

fn tone_style(theme: &Theme, tone: crate::agent_view::Tone) -> Style {
    use crate::agent_view::Tone as T;
    match tone {
        T::Normal => Style::default(),
        T::Dim => Style::default().fg(theme.color(Tone::Dim)),
        T::Path => Style::default()
            .fg(theme.color(Tone::Cyan))
            .add_modifier(Modifier::BOLD),
        T::Pass => Style::default().fg(theme.color(Tone::Green)),
        T::Fail => Style::default().fg(theme.color(Tone::Red)),
        T::Warn => Style::default().fg(theme.color(Tone::Amber)),
        T::Summary => Style::default().fg(theme.color(Tone::Dim)),
        T::Output => Style::default(),
    }
}

/// Agent view: one dim speaker label per turn, one blank row between turns,
/// newest at the bottom. It follows the tail until the reader scrolls away,
/// with the chat transcript's reading-position rules.
fn draw_agent(
    frame: &mut Frame,
    area: Rect,
    tasks: &TaskControl,
    view: &crate::agent_view::View,
    theme: &Theme,
) {
    use crate::agent_view::{self as agent, Origin, Target};
    let (turns, title) = match view.target {
        Target::Task(root) => {
            let attempts = agent::gather(tasks, root);
            let notes = tasks.owner.notes(view.target);
            let budget = tasks
                .autopilot
                .as_ref()
                .filter(|_| tasks.autopilot_roots.contains(&root))
                .map(|status| status.max_repairs);
            (
                agent::project(&attempts, &notes, view.expanded),
                agent::title(&attempts, budget),
            )
        }
        Target::Architect => {
            let planner = &tasks.planner;
            let origin = if tasks
                .autopilot
                .as_ref()
                .is_some_and(|status| status.state == crate::autopilot::RunState::Planning)
            {
                Origin::Autopilot
            } else {
                Origin::You
            };
            let request = if view.expanded {
                planner.prompt().to_owned()
            } else {
                // Validation retries follow the goal; the goal is the request.
                planner
                    .prompt()
                    .split(" | ")
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            };
            let draft = if planner.active() || planner.checkpoint().is_some() {
                planner.preview()
            } else {
                String::new()
            };
            let state = if planner.active() {
                "planning"
            } else if planner.checkpoint().is_some() {
                "draft"
            } else {
                "idle"
            };
            (
                agent::project_architect(
                    Some(&request),
                    origin,
                    &draft,
                    &tasks.owner.notes(view.target),
                    view.expanded,
                ),
                format!("Agent · architect · {state}"),
            )
        }
    };
    let label = Style::default().fg(theme.color(Tone::Dim));
    let mut lines = Vec::new();
    let mut blocks = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        let start = lines.len();
        if index > 0 {
            lines.push(Line::default());
        }
        lines.push(Line::styled(safe(&turn.label), label));
        let room = usize::from(area.width.saturating_sub(4));
        for (text, tone) in &turn.lines {
            let text = match tone {
                agent::Tone::Summary | agent::Tone::Output => truncate(&safe(text), room),
                _ => safe(text),
            };
            lines.push(Line::styled(text, tone_style(theme, *tone)));
        }
        blocks.push(crate::reading::Block {
            key: crate::reading::BlockKey::Message(index),
            start,
            len: lines.len() - start,
        });
    }
    if lines.is_empty() {
        lines.push(Line::styled("Nothing recorded for this agent yet.", label));
    }
    frame.render_widget(Clear, area);
    let block = frame_block(area, format!(" {} ", safe(&title)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
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
    let position = view.position(&heights, inner.height, &blocks);
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(position.line).collect::<Vec<_>>())
            .wrap(Wrap { trim: false })
            .scroll((position.row.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
}

fn draw_transcript(frame: &mut Frame, app: &App, tasks: Option<&TaskControl>, area: Rect) {
    let session = &app.sessions[app.selected];
    let identity = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    // `lines` is the current run of non-message rows; message bodies come from the
    // session's cache and are never copied here. `base` counts rows before the run.
    let mut lines = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();
    let mut base = 0usize;
    let mut blocks: Vec<crate::reading::Block> = Vec::new();
    let mut cache = session.transcript_cache.borrow_mut();
    cache.sync(
        session
            .messages
            .iter()
            .map(|m| (m.content.as_str(), m.role != "user")),
    );
    // A new chat suggests the common path, even below the workspace arrival line.
    if session.messages.is_empty()
        && session.task_receipts().is_empty()
        && session.commands().iter().all(|command| {
            matches!(
                command.intent,
                crate::command_intent::Intent::SelectionArrival { .. }
            )
        })
    {
        lines.push(Line::from(
            "Type /go GOAL and autopilot plans, runs and reviews it (or /plan GOAL, review, /go).",
        ));
        lines.push(Line::from("Or chat with your local model."));
        lines.push(Line::default());
    }
    // Compaction (issue #187): the summary note stands where the oldest turns were.
    if let Some(note) = session.compaction() {
        lines.push(Line::styled(format!("▸ {}", note.marker()), dim()));
        lines.extend(
            note.summary
                .lines()
                .map(|line| Line::styled(format!("  {}", safe(line)), dim())),
        );
        lines.push(Line::default());
    }
    // Project notes arrival (issue #322): display only, not a message.
    if !session.notes_lines().is_empty() {
        lines.extend(
            session
                .notes_lines()
                .iter()
                .map(|line| Line::styled(safe(line), dim())),
        );
        lines.push(Line::default());
    }
    let mut receipts = session.task_receipts().iter().peekable();
    // A Wayfinder preparation can finish after a later command was admitted.
    // Render at its saved user-turn boundary, preserving sequence within a boundary.
    let mut ordered_commands: Vec<_> = session.commands().iter().collect();
    ordered_commands.sort_unstable_by_key(|command| (command.after_messages, command.sequence));
    let mut commands = ordered_commands.into_iter().peekable();
    let wayfinder_requests: std::collections::BTreeMap<_, _> = session
        .commands()
        .iter()
        .filter_map(|command| match &command.intent {
            crate::command_intent::Intent::Wayfinder {
                request,
                user_message,
            } => Some((*user_message, request)),
            _ => None,
        })
        .collect();
    // Consecutive automatic steps for one task share one line; only the
    // latest such line can still grow.
    // (task, first line, block index, phases); the group is always last.
    let mut group: Option<(Option<u64>, usize, usize, Vec<Phase>)> = None;
    let width = usize::from(area.width.saturating_sub(4)).max(20);
    for index in 0..=session.messages.len() {
        loop {
            let receipt = receipts.peek().filter(|item| item.after_messages == index);
            let command = commands.peek().filter(|item| item.after_messages == index);
            let receipt_first = match (receipt, command) {
                (Some(receipt), Some(command)) => receipt.sequence < command.sequence,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let mut start = lines.len();
            let key = if receipt_first {
                let reference = receipts.next().unwrap();
                group = None;
                lines.extend(task_receipt_lines(reference, identity));
                crate::reading::BlockKey::TaskReceipt(reference.revision)
            } else {
                let command = commands.next().unwrap();
                match collapsed_step(command, session.commands(), tasks) {
                    Some(step) if step.phases.is_empty() => {}
                    Some(step) => {
                        let repeated = group
                            .as_ref()
                            .filter(|(task, ..)| task.is_none() && step.task.is_none())
                            .filter(|_| command.text.starts_with("Autopilot · "))
                            .and_then(|(.., phases)| repeated_failure(phases, &step.phases));
                        match group.as_mut() {
                            Some((None, at, block, phases)) if repeated.is_some() => {
                                *phases = repeated.unwrap();
                                lines.truncate(*at);
                                lines.extend(collapsed_lines(None, phases, width));
                                lines.push(Line::default());
                                blocks[*block].len = lines.len() - *at;
                                start = lines.len();
                            }
                            Some((Some(task), at, block, phases)) if step.task == Some(*task) => {
                                phases.extend(step.phases);
                                lines.truncate(*at);
                                lines.extend(collapsed_lines(Some(*task), phases, width));
                                lines.push(Line::default());
                                blocks[*block].len = lines.len() - *at;
                                start = lines.len();
                            }
                            previous => {
                                // Consecutive collapsed lines share one trailing blank row.
                                if previous.is_some()
                                    && lines.last().is_some_and(|l| l.width() == 0)
                                {
                                    lines.pop();
                                }
                                start = lines.len();
                                lines.extend(collapsed_lines(step.task, &step.phases, width));
                                lines.push(Line::default());
                                group = Some((step.task, start, blocks.len(), step.phases));
                            }
                        }
                    }
                    None => {
                        group = None;
                        lines.extend(command_lines(command, tasks));
                    }
                }
                crate::reading::BlockKey::Command(command.sequence)
            };
            blocks.push(crate::reading::Block {
                key,
                start: base + start,
                len: lines.len().saturating_sub(start),
            });
        }
        let Some(message) = session.messages.get(index) else {
            break;
        };
        group = None;
        // An empty reply is told by the failure row below, not shown as a blank block
        // (issue #397).
        if index + 1 == session.messages.len()
            && message.role != "user"
            && message.content.trim().is_empty()
            && session.tool_steps(index).is_empty()
            && session.status == Status::Failed(crate::model::NO_TEXT.into())
        {
            continue;
        }
        let (heading, color) = if message.role == "user" {
            ("You".into(), Color::DarkGray)
        } else {
            response_heading(
                session.source(index),
                tasks.and_then(|tasks| tasks.canonical_scope.as_ref()),
                wayfinder_requests.get(&index.saturating_sub(1)).copied(),
            )
        };
        let heading = if session.interrupted_turn(index - index % 2) {
            let kept = session.applied_note(index - index % 2 + 1).is_some();
            let sent = if kept {
                "applied changes sent as context"
            } else {
                "not sent as context"
            };
            format!("{heading} · interrupted, {sent}")
        } else {
            heading
        };
        // Speaker labels are quiet; only an unverified claim keeps its warning colour.
        let color = if color == Color::Green {
            Color::DarkGray
        } else {
            color
        };
        // Tool rounds (issue #233) show before the reply they fed: call, then a cut result.
        let mut first_call = 0;
        for step in session.tool_steps(index) {
            lines.extend(tool_step_lines(
                step,
                width,
                session.tool_results_expanded,
                |at| session.rule_of(index, first_call + at),
            ));
            first_call += step.calls.len();
        }
        base += lines.len();
        segments.push(Segment::Run(std::mem::take(&mut lines)));
        let body = cache.entry(index).lines.len();
        blocks.push(crate::reading::Block {
            key: crate::reading::BlockKey::Message(index),
            start: base,
            len: body + 2,
        });
        base += body + 2;
        segments.push(Segment::Message {
            index,
            heading: Line::styled(heading, Style::default().fg(color)),
        });
    }
    // A failed request keeps its reason in the transcript until the next attempt.
    if let Status::Failed(error) = &session.status {
        // The empty-reply text already names Ctrl+R (issue #397).
        let text = if error == crate::model::NO_TEXT {
            format!("✗ {error}")
        } else {
            format!("✗ Request failed · {} · Ctrl+R retries", single_line(error))
        };
        lines.push(Line::styled(text, Style::default().fg(Color::Red)));
    }
    segments.push(Segment::Run(lines));
    let block = frame_block(
        area,
        format!(" Chat {} · {} ", app.selected + 1, session.short_status()),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut metadata = capacity_wait_lines(
        session,
        &app.pane.theme,
        Instant::now(),
        usize::from(inner.width),
    );
    metadata.extend(timing_line(session, app.num_ctx).map(|text| Line::styled(text, dim())));
    if !metadata.is_empty() {
        metadata.push(Line::default());
    }
    let metadata = Paragraph::new(metadata).wrap(Wrap { trim: false });
    let metadata_height = metadata
        .line_count(inner.width)
        .min(inner.height.saturating_sub(1) as usize) as u16;
    let content =
        Layout::vertical([Constraint::Length(metadata_height), Constraint::Min(1)]).split(inner);
    frame.render_widget(metadata, content[0]);
    if content[1].width > 0 && content[1].height > 0 {
        let width = content[1].width;
        let mut heights = Vec::new();
        for segment in &segments {
            match segment {
                Segment::Run(run) => heights.extend(
                    run.iter()
                        .map(|line| crate::transcript_cache::line_height(line, width)),
                ),
                Segment::Message { index, heading } => {
                    cache.heights(*index, width);
                    heights.push(crate::transcript_cache::line_height(heading, width));
                    heights.extend_from_slice(&cache.entry(*index).heights);
                    heights.push(1);
                }
            }
        }
        // The pending card is sized to the viewport so its header, args and key
        // line all fit; `card_head` is its first line's index and the args offset.
        if !app.transient_rows.is_empty() {
            let rows: Vec<Line<'static>> = app
                .transient_rows
                .iter()
                .map(|row| Line::styled(safe(row), dim()))
                .collect();
            heights.extend(
                rows.iter()
                    .map(|line| crate::transcript_cache::line_height(line, width)),
            );
            segments.push(Segment::Run(rows));
        }
        let mut card_head = None;
        if let Some(card) = session.pending_approval() {
            let (card_lines, head) =
                approval_card_lines(card, usize::from(width), usize::from(content[1].height));
            card_head = Some((heights.len(), head));
            heights.extend(
                card_lines
                    .iter()
                    .map(|line| crate::transcript_cache::line_height(line, width)),
            );
            segments.push(Segment::Run(card_lines));
        }
        let position = session.reading_position_blocks(&heights, content[1].height, &blocks);
        // Arming waits for the header and args rows to be inside the viewport.
        session.set_approval_header_drawn(card_head.is_some_and(|(at, head)| {
            let top = heights[..position.line.min(heights.len())]
                .iter()
                .sum::<usize>()
                + position.row;
            let first = heights[..(at + 1).min(heights.len())].iter().sum::<usize>();
            let last = heights[..(at + head + 1).min(heights.len())]
                .iter()
                .sum::<usize>();
            first >= top && last <= top + usize::from(content[1].height)
        }));
        // Only rows that can reach the viewport are materialised; wrapping them is
        // identical to wrapping the whole tail. Slicing logical lines before the
        // widget's u16 scroll keeps long bounded responses navigable beyond 65,535 rows.
        let wanted = position.row.saturating_add(usize::from(content[1].height));
        let mut window: Vec<Line<'static>> = Vec::new();
        let mut covered = 0usize;
        let mut at = 0usize;
        let mut done = false;
        let mut take = |line: &Line<'static>, at: &mut usize, cut: bool| -> bool {
            if *at >= position.line && covered <= wanted {
                covered += heights[*at];
                window.push(if cut {
                    crate::transcript_cache::cut_line(line, width)
                } else {
                    line.clone()
                });
            }
            *at += 1;
            covered > wanted
        };
        for segment in &segments {
            match segment {
                Segment::Run(run) => {
                    for line in run {
                        done |= take(line, &mut at, false);
                    }
                }
                Segment::Message { index, heading } => {
                    done |= take(heading, &mut at, false);
                    let entry = cache.entry(*index);
                    for (line, cut) in entry.lines.iter().zip(&entry.cut) {
                        done |= take(line, &mut at, *cut);
                    }
                    done |= take(&Line::default(), &mut at, false);
                }
            }
            if done {
                break;
            }
        }
        frame.render_widget(
            Paragraph::new(window)
                .wrap(Wrap { trim: false })
                .scroll((position.row.min(u16::MAX as usize) as u16, 0)),
            content[1],
        );
    }
}

/// A stretch of the transcript: rows built this frame, or a cached message body
/// framed by its heading and a trailing blank row.
enum Segment {
    Run(Vec<Line<'static>>),
    Message {
        index: usize,
        heading: Line<'static>,
    },
}

/// The cat running along its dotted track beside the capacity-wait text, or
/// stacked above it when the pane is too narrow for both; text alone when even
/// the cat does not fit. Every line is at most `width` columns wide.
fn capacity_wait_lines(
    session: &crate::model::Session,
    theme: &Theme,
    now: Instant,
    width: usize,
) -> Vec<Line<'static>> {
    let Some(live) = session.capacity_wait() else {
        return Vec::new();
    };
    let text = format!("Waiting for another Alfredo process (capacity {live}) · Esc cancel");
    let track = theme.capacity_cat(now, width);
    let cat = Style::default().fg(theme.color(Tone::Amber));
    if track.is_empty() {
        vec![Line::styled(text, dim())]
    } else if track.width() + 2 + text.width() <= width {
        vec![Line::from(vec![
            Span::styled(track, cat),
            Span::raw("  "),
            Span::styled(text, dim()),
        ])]
    } else {
        vec![Line::styled(track, cat), Line::styled(text, dim())]
    }
}

/// One dim line: queue position or elapsed time, plus generation speed.
fn timing_line(session: &crate::model::Session, num_ctx: Option<u32>) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(observation) = session.queue_observation() {
        parts.push(format!(
            "queued {}/{} · {}/{} active",
            observation.position, observation.waiting, observation.active, observation.capacity
        ));
    } else if let Some(phase) = session
        .wait_phase()
        .filter(|_| session.capacity_wait().is_none())
    {
        parts.push(phase.into());
    }
    if let Some(timing) = &session.timing {
        parts.push(timing.compact(std::time::Instant::now()));
    }
    if session.wait_phase() == Some("running run_command") {
        parts.push("Esc stops".into());
    }
    if let Some(metrics) = &session.metrics {
        parts.extend(metrics.context_fill(num_ctx));
        if let (Some(count), Some(duration)) = (
            metrics.eval_count,
            metrics.eval_duration.filter(|value| *value > 0),
        ) {
            parts.push(format!(
                "{:.0} tok/s",
                count as f64 / (duration as f64 / 1e9)
            ));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Greedy word wrap keeping runs of spaces inside a line; words longer than
/// the width are split. Continuation lines never start with spaces.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for word in text.trim().split(' ') {
        if used > 0 && word.is_empty() {
            if used < width {
                current.push(' ');
                used += 1;
            }
            continue;
        }
        let size = word.width();
        if used > 0 && used + 1 + size > width {
            lines.push(std::mem::take(&mut current));
            used = 0;
        }
        if size > width {
            for character in word.chars() {
                let cell = unicode_width::UnicodeWidthChar::width(character).unwrap_or(0);
                if used + cell > width {
                    lines.push(std::mem::take(&mut current));
                    used = 0;
                }
                current.push(character);
                used += cell;
            }
            continue;
        }
        if used > 0 {
            current.push(' ');
            used += 1;
        }
        current.push_str(word);
        used += size;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

/// The pending-action card (issue #235): tool, arguments, diff preview and keys.
fn approval_card_lines(
    card: &crate::chat_tools::approval::Card,
    width: usize,
    rows: usize,
) -> (Vec<Line<'static>>, usize) {
    use crate::chat_tools::approval::CardKind;
    let warn = Style::default().fg(Color::Yellow);
    // A host-escalated run_command (#236) gets its own colour and a HOST banner.
    let (header, style) = match card.kind {
        CardKind::Push => (
            format!(
                "! PUSH {} · {}",
                card.tool,
                dashboard::single_line_visible(&card.summary)
            ),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        CardKind::Edit | CardKind::Git | CardKind::Shell { .. } => {
            let color = if card.escalated() {
                Color::LightRed
            } else {
                Color::Yellow
            };
            (
                format!(
                    "? Approve {} · {}",
                    card.tool,
                    dashboard::single_line_visible(&card.summary)
                ),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )
        }
    };
    let mut lines = vec![Line::default(), Line::styled(header, style)];
    if card.kind == CardKind::Push {
        lines.push(Line::styled(
            "  publishes to a remote · asked every time, never always-allowed",
            Style::default().fg(Color::Red),
        ));
    }
    if card.tool == "run_command"
        && card
            .arguments
            .get("command")
            .and_then(|c| c.as_str())
            .is_some_and(crate::chat_tools::allow::pushes)
    {
        lines.push(Line::styled(
            "  push: asked every time, never always-allowed",
            Style::default().fg(Color::Red),
        ));
    }
    if card.escalated() {
        lines.push(Line::styled(
            "  ! HOST · network on · your environment and credentials (SSH agent, tokens) · runs outside the sandbox",
            style.add_modifier(Modifier::REVERSED),
        ));
    }
    let arguments = card.arguments.to_string();
    let shown: String = arguments.chars().take(width.max(20) * 2).collect();
    lines.push(Line::styled(
        format!("  args {}", safe_visible(&shown)),
        dim(),
    ));
    // The header, args and key line always fit: a preview taller than the rows
    // left shows its first lines and a count of the rest (issue #268).
    let head = lines.len() - 1;
    let fixed: usize = lines
        .iter()
        .map(|line| crate::transcript_cache::line_height(line, width as u16))
        .sum::<usize>()
        + 1;
    let mut budget = rows.saturating_sub(fixed);
    // Tabs show as `→` plus padding, so a tab-only change never reads as no change
    // (issue #404); the stops count from the `+`/`-` column, as in `git diff`.
    let preview: Vec<(&str, String)> = card
        .preview
        .lines()
        .map(|line| {
            // Only `+`/`-` diff lines mark tabs and escape a literal arrow; notes such as
            // `line endings LF → CRLF` and the `@@` header keep their text.
            let shown = safe_visible(line);
            let shown = if line.starts_with(['+', '-']) {
                dashboard::show_tabs(&shown)
            } else if line.starts_with("$ ") {
                // A run_command line is drawn as `a` shows it (issue #409).
                crate::chat_tools::allow::show_line(line)
            } else {
                dashboard::expand_tabs(&shown)
            };
            (line, shown)
        })
        .collect();
    let heights: Vec<usize> = preview
        .iter()
        .map(|(_, shown)| (shown.chars().count() + 2).div_ceil(width.max(1)).max(1))
        .collect();
    let mut shown = preview.len();
    if heights.iter().sum::<usize>() > budget {
        // Keep one row for the count of what is cut.
        budget = budget.saturating_sub(1);
        shown = 0;
        let mut used = 0;
        for height in &heights {
            if used + height > budget {
                break;
            }
            used += height;
            shown += 1;
        }
    }
    for (line, text) in &preview[..shown] {
        let color = if line.starts_with('+') {
            Color::Green
        } else if line.starts_with('-') {
            Color::Red
        } else {
            Color::DarkGray
        };
        lines.push(Line::styled(
            format!("  {text}"),
            Style::default().fg(color),
        ));
    }
    if shown < preview.len() {
        lines.push(Line::styled(
            format!("  [{} more diff lines]", preview.len() - shown),
            dim(),
        ));
    }
    lines.push(Line::styled(
        match crate::chat_tools::allow::offer(card) {
            Some(offer) => format!(
                "  y approve · {} · n deny · Esc cancels the turn (deny)",
                safe_visible(&offer)
            ),
            None => match crate::chat_tools::allow::unavailable_reason(card) {
                Some(why) => format!(
                    "  y approve · n deny · Esc cancels the turn (deny) · a not offered: {}",
                    safe_visible(&why)
                ),
                None => "  y approve · n deny · Esc cancels the turn (deny)".into(),
            },
        },
        warn,
    ));
    (lines, head)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Session, Update};

    #[test]
    fn capacity_wait_lines_clip_the_cat_to_the_width_and_drop_it_when_it_cannot_fit() {
        let mut session = Session::new("fixture".into());
        session.insert("hi");
        session.begin().unwrap();
        let theme = Theme {
            motion: false,
            ..Theme::default()
        };
        let now = Instant::now();
        assert!(capacity_wait_lines(&session, &theme, now, 80).is_empty());
        session.apply(1, Update::CapacityWait { live: 2 });
        let lines = |width| capacity_wait_lines(&session, &theme, now, width);
        // The text wraps in its paragraph; only the cat track is sized here.
        for width in 0..140 {
            for line in lines(width) {
                let track = line
                    .spans
                    .first()
                    .filter(|span| !span.content.contains("Waiting"));
                assert!(
                    track.is_none_or(|span| span.content.width() <= width),
                    "{width}"
                );
            }
        }
        // One line with room, stacked when tight, text alone below the cat's width.
        assert_eq!(lines(120).len(), 1);
        assert_eq!(lines(40).len(), 2);
        assert_eq!(lines(20).len(), 2);
        assert_eq!(lines(20)[0].width(), 20);
        assert_eq!(lines(12).len(), 1);
        assert!(!lines(12)[0].to_string().contains("=^"));
    }
}

#[cfg(test)]
mod fit_words_tests {
    use super::{fit_words, inspector::next_action};

    #[test]
    fn notices_cut_at_a_word_boundary_within_the_width() {
        assert_eq!(fit_words("short note", 20), "short note");
        let cut = fit_words("Unknown command /help · F1 lists commands", 30);
        assert_eq!(cut, "Unknown command /help · F1…");
        assert!(unicode_width::UnicodeWidthStr::width(cut.as_str()) <= 30);
    }

    fn review_snapshot(criteria: &[&str]) -> crate::tasks::Snapshot {
        use crate::tasks::{Action, Receipt, Request, WorkPolicy};
        let step = crate::planner::Step {
            title: "Task".into(),
            acceptance: criteria.iter().map(|c| c.to_string()).collect(),
            model: "fixture".into(),
            dependencies: vec![],
            policy: WorkPolicy {
                files: vec!["a.rs".into()],
                check: vec!["true".into()],
            },
        };
        crate::tasks::Snapshot {
            schema_version: crate::tasks::SCHEMA_VERSION,
            workspace: std::path::PathBuf::from("."),
            mission: "mission".into(),
            revision: 1,
            tasks: vec![crate::tasks::Task {
                id: 1,
                title: "Task".into(),
                model: "fixture".into(),
                dependencies: vec![],
                status: crate::tasks::TaskStatus::ReviewReady,
                policy: None,
                repair_of: None,
                run: None,
            }],
            receipts: vec![Receipt {
                task: 1,
                revision: 1,
                request: Request {
                    correlation: "plan".into(),
                    expected_revision: 0,
                    action: Action::Plan {
                        plan: crate::planner::Plan {
                            usage: None,
                            prompt: "p".into(),
                            planner: "fixture".into(),
                            context: None,
                            scope: None,
                            architecture: None,
                            tasks: vec![step],
                        },
                    },
                },
            }],
        }
    }

    #[test]
    fn fit_words_keeps_the_start_of_a_long_unbroken_token() {
        let cut = fit_words(r#"Use /review 4 {"outcome":"approved","reason":"…"}"#, 40);
        assert!(
            cut.starts_with(r#"Use /review 4 {"outcome":"appr"#),
            "{cut}"
        );
    }

    #[test]
    fn review_ready_hint_matches_what_accept_allows() {
        let with = review_snapshot(&["Returns 42"]);
        let hint = next_action(&with, &with.tasks[0]).unwrap();
        assert!(
            hint.contains("/review 1 JSON") && hint.contains("/reject 1"),
            "{hint}"
        );
        assert!(!hint.contains("/accept"), "{hint}");
        let without = review_snapshot(&[]);
        let hint = next_action(&without, &without.tasks[0]).unwrap();
        assert!(
            hint.contains("/accept 1") && hint.contains("/reject 1"),
            "{hint}"
        );
    }
}

/// `running` only while a worker or owner lock can still be live; a run whose
/// process is gone reads `interrupted`.
fn task_state_word(tasks: &TaskControl, task: &crate::tasks::Task) -> &'static str {
    if tasks.interrupted(task.id) {
        "interrupted"
    } else {
        dashboard::state_word(task)
    }
}
