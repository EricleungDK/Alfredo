use crate::dashboard::{self, single_line, truncate};
use crate::model::{App, Status};
use crate::side_pane::{self, MissionRow, Projection, RowKind, Section, WorkRow};
use crate::task_control::TaskControl;
use crate::theme::{ColorMode, Record, RowStatus, Theme, Tone};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame,
};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

/// Below this width the side pane collapses to one summary row (F6 overlay).
pub const PANE_BREAKPOINT: u16 = 88;
/// Detail label column; `Criteria ` is the longest label.
const LABEL: usize = 9;
/// Tree indentation stops at group › task › repair; repairs of repairs stay
/// at the repair level (their titles name their parent).
const MAX_DEPTH: usize = 2;
/// Cells of an automatic command shown in the transcript.
const COMMAND_SUMMARY: usize = 160;

fn safe(text: &str) -> String {
    dashboard::safe(text)
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
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
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(autopilot.is_some())),
        Constraint::Min(3),
        Constraint::Length(3),
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
    let help = app.completion.as_ref().filter(|completion| {
        completion
            .choices
            .iter()
            .any(|choice| choice.group.is_some())
    });
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
        Some((tasks, target)) => crate::agent_view::prompt_title(tasks, target),
        None => " Prompt · Enter send ".to_string(),
    };
    let draft = session.draft_view(rows[3].width.saturating_sub(4) as usize);
    frame.render_widget(
        Paragraph::new(safe(&draft)).block(
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
    } else if tasks.is_some_and(|tasks| tasks.visible) {
        dashboard::Footer::Tasks
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
                single_line(&note),
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
            let tone = if health.healthy() {
                Tone::Green
            } else {
                Tone::Red
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
            .title(" Help · ↑↓ choose · Enter fills prompt · Esc close ")
            .title_bottom(
                " F2 tasks/chat  F3 evidence  F4 activity  F5 pause  F6 pane  Enter agent  ^O expand ",
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
                safe(&tasks.planner.preview())
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
                if *name == session.model { "›" } else { " " },
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
                    " Complete · ↑↓ choose · Enter fills draft · Esc close ".into(),
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
            (
                agent::project(&attempts, &notes, view.expanded),
                agent::title(&attempts),
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
    cache.sync(session.messages.iter().map(|m| m.content.as_str()));
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
            "Type /go GOAL and autopilot plans, runs and reviews it.",
        ));
        lines.push(Line::from("Or chat with your local model."));
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
                    Some(step) => match group.as_mut() {
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
                            if previous.is_some() && lines.last().is_some_and(|l| l.width() == 0) {
                                lines.pop();
                            }
                            start = lines.len();
                            lines.extend(collapsed_lines(step.task, &step.phases, width));
                            lines.push(Line::default());
                            group = Some((step.task, start, blocks.len(), step.phases));
                        }
                    },
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
        let (heading, color) = if message.role == "user" {
            ("You".into(), Color::DarkGray)
        } else {
            response_heading(
                session.source(index),
                tasks.and_then(|tasks| tasks.canonical_scope.as_ref()),
                wayfinder_requests.get(&index.saturating_sub(1)).copied(),
            )
        };
        // Speaker labels are quiet; only an unverified claim keeps its warning colour.
        let color = if color == Color::Green {
            Color::DarkGray
        } else {
            color
        };
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
        lines.push(Line::styled(
            format!("✗ Request failed · {} · Ctrl+R retries", single_line(error)),
            Style::default().fg(Color::Red),
        ));
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
    metadata.extend(timing_line(session).map(|text| Line::styled(text, dim())));
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
        let position = session.reading_position_blocks(&heights, content[1].height, &blocks);
        // Only rows that can reach the viewport are materialised; wrapping them is
        // identical to wrapping the whole tail. Slicing logical lines before the
        // widget's u16 scroll keeps long bounded responses navigable beyond 65,535 rows.
        let wanted = position.row.saturating_add(usize::from(content[1].height));
        let mut window: Vec<Line<'static>> = Vec::new();
        let mut covered = 0usize;
        let mut at = 0usize;
        let mut done = false;
        let mut take = |line: &Line<'static>, at: &mut usize| -> bool {
            if *at >= position.line && covered <= wanted {
                covered += heights[*at];
                window.push(line.clone());
            }
            *at += 1;
            covered > wanted
        };
        for segment in &segments {
            match segment {
                Segment::Run(run) => {
                    for line in run {
                        done |= take(line, &mut at);
                    }
                }
                Segment::Message { index, heading } => {
                    done |= take(heading, &mut at);
                    for line in &cache.entry(*index).lines {
                        done |= take(line, &mut at);
                    }
                    done |= take(&Line::default(), &mut at);
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
fn timing_line(session: &crate::model::Session) -> Option<String> {
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
    if let Some(metrics) = &session.metrics {
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
fn next_action(snapshot: &crate::tasks::Snapshot, task: &crate::tasks::Task) -> Option<String> {
    use crate::tasks::TaskStatus;
    let id = task.id;
    match task.status {
        TaskStatus::Proposed if task.policy.is_none() => Some(format!("/permit {id} JSON")),
        TaskStatus::Proposed => Some(format!("/approve {id}")),
        TaskStatus::Approved if task.policy.is_none() => Some(format!("/permit {id} JSON")),
        TaskStatus::Approved => Some(format!("/run {id}")),
        TaskStatus::ReviewReady => Some(format!("/accept {id}{GAP}/reject {id}")),
        TaskStatus::NeedsHumanReview => Some(format!("/review {id} JSON")),
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
fn work_inspector(
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
            format!(" Task #{} · {} ", task.id, dashboard::state_word(task)),
            true,
            pinned,
        );
    }
    (
        lines,
        format!(" Task #{} · {} ", task.id, dashboard::state_word(task)),
        false,
        0,
    )
}

/// Slice logical lines before Ratatui's u16 scroll limit. The full row offset is
/// local UI state; drawing and navigation never mutate canonical task records.
/// A live panel follows its tail until the user scrolls away from it.
fn task_panel(
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
fn task_panel_pinned(
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
fn response_heading(
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

fn verified_receipt<'a>(
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
fn task_receipt_lines(
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

fn receipt_style(receipt: &crate::tasks::Receipt) -> (&'static str, Color) {
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
enum StepKind {
    Done,
    Failed,
    Stopped,
    Pending,
    /// A repair was proposed; the line's glyph is its task's own outcome.
    Repair,
}
type Phase = (String, StepKind);

struct Step {
    task: Option<u64>,
    /// Empty: nothing worth a line (such as an acknowledged `/dispatch on`).
    phases: Vec<Phase>,
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
fn collapsed_step(
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
                            format!("plan failed: {}", single_line(reason)),
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

/// One collapsed entry, wrapped at word boundaries with a hanging indent.
fn collapsed_lines(task: Option<u64>, phases: &[Phase], width: usize) -> Vec<Line<'static>> {
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
fn command_lines(
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
