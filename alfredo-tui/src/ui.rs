use crate::dashboard::{self, single_line, truncate};
use crate::model::{App, Status};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthStr;

fn safe(text: &str) -> String {
    dashboard::safe(text)
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// A bordered block only when the area can hold a complete box.
fn frame_block(area: Rect, title: String) -> Block<'static> {
    if area.height < 3 || area.width < 4 {
        Block::default()
    } else {
        Block::bordered().title(title)
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    draw_inner(frame, app, None);
}

pub fn draw_with_tasks(frame: &mut Frame, app: &App, tasks: &crate::task_control::TaskControl) {
    draw_inner(frame, app, Some(tasks));
}

fn draw_inner(frame: &mut Frame, app: &App, tasks: Option<&crate::task_control::TaskControl>) {
    let area = frame.area();
    if area.width < 32 || area.height < 10 {
        frame.render_widget(
            Paragraph::new("Alfredo\nResize to at least 32 × 10\nCtrl+Q quit")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let identity = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    let autopilot = tasks.and_then(|tasks| tasks.autopilot.as_ref());
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(identity.is_some()) + u16::from(autopilot.is_some())),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    let session = &app.sessions[app.selected];
    let active = app.sessions.iter().filter(|s| s.status.active()).count();
    let dashboard_visible = tasks.is_some_and(|tasks| tasks.visible);
    draw_status_row(frame, app, tasks, rows[0], identity.is_some(), active);
    let header = Layout::vertical([
        Constraint::Length(u16::from(identity.is_some())),
        Constraint::Length(u16::from(autopilot.is_some())),
    ])
    .split(rows[1]);
    if let Some(snapshot) = identity {
        draw_mission_row(frame, app, snapshot, header[0]);
    }
    if let Some(status) = autopilot {
        frame.render_widget(
            Paragraph::new(dashboard::autopilot_row(
                status,
                usize::from(header[1].width),
            ))
            .style(Style::default().fg(Color::Yellow)),
            header[1],
        );
    }
    let help = app.completion.as_ref().filter(|completion| {
        completion
            .choices
            .iter()
            .any(|choice| choice.group.is_some())
    });
    if let Some(help) = help {
        draw_help(frame, help, rows[2]);
    } else {
        draw_body(frame, app, tasks, rows[2]);
    }
    let draft = session.draft_view(rows[3].width.saturating_sub(2) as usize);
    frame.render_widget(
        Paragraph::new(safe(&draft)).block(
            Block::bordered()
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Prompt · Enter send "),
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
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                dashboard::footer_hints(dashboard_visible, usize::from(area.width)),
                dim(),
            ),
            Line::styled(single_line(&note), Style::default().fg(Color::Yellow)),
        ]),
        rows[4],
    );
}

fn draw_status_row(
    frame: &mut Frame,
    app: &App,
    tasks: Option<&crate::task_control::TaskControl>,
    area: Rect,
    mission_row: bool,
    active: usize,
) {
    let session = &app.sessions[app.selected];
    // With a mission row, health sits at its right edge so this row keeps its width.
    let health = if mission_row {
        Span::default()
    } else {
        health_span(app, area.width)
    };
    let used = u16::try_from(health.width()).unwrap_or(u16::MAX);
    let text = if let Some(tasks) = tasks {
        let dispatch = if area.width >= 60 {
            format!(
                "dispatch {} · ",
                if tasks.dispatch.enabled { "on" } else { "off" }
            )
        } else {
            String::new()
        };
        let status = tasks
            .work_status()
            .concise(area.width.saturating_sub(10 + used + dispatch.len() as u16));
        let mut header = format!(" {dispatch}{status}");
        let remaining =
            usize::from(area.width.saturating_sub(used)).saturating_sub(9 + header.width());
        if remaining >= 16 && !tasks.visible {
            header.push_str(&format!(" · {active} chats · {}", safe(&session.model)));
            if tasks.scope_status.label.starts_with("Wayfinder /") {
                header.push_str(&format!(" · {}", safe(&tasks.scope_status.label)));
            }
        }
        header
    } else {
        format!(
            "  Conversations · {active} active · {}",
            safe(&session.model)
        )
    };
    let room = usize::from(area.width.saturating_sub(used)).saturating_sub(9);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " ALFREDO ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            health,
            Span::raw(truncate(&text, room)),
        ])),
        area,
    );
}

fn health_span(app: &App, width: u16) -> Span<'static> {
    let session = &app.sessions[app.selected];
    let health = app.health.state(&session.model);
    health
        .label(&session.model, width >= 100)
        .map(|label| {
            let color = if health.healthy() {
                Color::Green
            } else {
                Color::Red
            };
            Span::styled(format!(" {label} "), Style::default().fg(color))
        })
        .unwrap_or_default()
}

fn draw_mission_row(frame: &mut Frame, app: &App, snapshot: &crate::tasks::Snapshot, area: Rect) {
    let health = health_span(app, area.width);
    let health_width = u16::try_from(health.width())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let line =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(health_width)]).split(area);
    frame.render_widget(Paragraph::new(Line::from(health)), line[1]);
    let text = format!(
        " Mission: {} · {}",
        single_line(&snapshot.mission),
        single_line(&snapshot.workspace.display().to_string())
    );
    frame.render_widget(
        Paragraph::new(truncate(&text, usize::from(line[0].width)))
            .style(Style::default().fg(Color::Cyan)),
        line[0],
    );
}

/// F1: grouped command catalog over the whole body, scrolling with the selection.
fn draw_help(frame: &mut Frame, help: &crate::commands::Completion, area: Rect) {
    let width = usize::from(area.width.saturating_sub(4));
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
            .title(" Help · ↑↓ choose · Enter fills prompt · Esc close ")
            .title_bottom(" F2 dashboard · F3 evidence · F4 activity · F5 pause ")
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

fn draw_body(
    frame: &mut Frame,
    app: &App,
    tasks: Option<&crate::task_control::TaskControl>,
    body: Rect,
) {
    let session = &app.sessions[app.selected];
    let identity = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    let width = body.width;
    let side = width >= 60;
    let focused_review = app.models_visible
        || app.completion.is_some()
        || tasks.is_some_and(|tasks| {
            tasks.visible
                && (tasks.evidence.is_some()
                    || tasks.planner.visible
                    || tasks.scope_view.is_some()
                    || tasks.activity.is_some())
        });
    let work_tree = tasks
        .filter(|tasks| tasks.visible)
        .map(|tasks| tasks.work_tree());
    let compact_tree = !side && body.height < 7;
    let list_size = if side {
        if focused_review {
            if width < 100 {
                0
            } else {
                28
            }
        } else if work_tree.is_some() {
            if width >= 100 {
                40
            } else {
                (width * 2 / 5).clamp(26, 40)
            }
        } else if width >= 100 {
            20
        } else {
            16
        }
    } else if focused_review {
        0
    } else if work_tree.is_some() {
        if compact_tree {
            1
        } else {
            (body.height / 3).clamp(3, 8)
        }
    } else if body.height >= 8 {
        3
    } else {
        0
    };
    let panes = Layout::default()
        .direction(if side {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints([Constraint::Length(list_size), Constraint::Min(1)])
        .split(body);
    let (items, list_title, selected): (Vec<ListItem>, String, Option<usize>) =
        if let Some(tasks) = tasks.filter(|tasks| tasks.visible) {
            let tree = work_tree.as_ref().unwrap();
            let selected = tasks
                .focused_work_node()
                .and_then(|selected| tree.rows.iter().position(|row| row.id == selected));
            let row_width = usize::from(panes[0].width.saturating_sub(4));
            let items = tree
                .rows
                .iter()
                .map(|row| work_row(row, identity, row_width))
                .collect();
            let (done, total) = identity.map(dashboard::done_total).unwrap_or_default();
            let filter = if tasks.task_query.trim().is_empty() {
                String::new()
            } else {
                format!(" · {}/{} shown", tree.matched_tasks, tree.total_tasks)
            };
            (
                items,
                format!(" Mission Work · {done}/{total} done{filter} "),
                selected,
            )
        } else {
            (
                app.sessions
                    .iter()
                    .enumerate()
                    .map(|(index, s)| ListItem::new(format!("{} {}", index + 1, s.short_status())))
                    .collect(),
                " Sessions ".into(),
                Some(app.selected),
            )
        };
    if panes[0].width > 0 && panes[0].height > 0 {
        let list = List::new(items)
            .block(if compact_tree && work_tree.is_some() {
                Block::default()
            } else {
                frame_block(panes[0], list_title)
            })
            .highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("› ");
        frame.render_stateful_widget(
            list,
            panes[0],
            &mut ListState::default().with_selected(selected),
        );
    }
    if tasks.is_none_or(|tasks| !tasks.visible) && !app.models_visible && app.completion.is_none() {
        draw_transcript(frame, app, tasks, panes[1]);
    }
    if let Some(tasks) =
        tasks.filter(|tasks| tasks.visible && !app.models_visible && app.completion.is_none())
    {
        tasks.detail_live.set(false);
        if tasks.planner.visible {
            task_panel(
                frame,
                panes[1],
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
                panes[1],
                format!(
                    " Verified run evidence · task #{} · /tasks to return ",
                    evidence.task
                ),
            );
            let inner = block.inner(panes[1]);
            frame.render_widget(Clear, panes[1]);
            frame.render_widget(block, panes[1]);
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
            frame.render_widget(Clear, panes[1]);
            let mut lines = vec![
                Line::from("Newest saved receipts first · /activity query · /activity #ID"),
                Line::from("/tasks returns to details · PgUp/PgDn scroll · /refresh reloads"),
                Line::default(),
            ];
            if let Some(snapshot) = &tasks.snapshot {
                let entries = crate::activity::entries(snapshot, query);
                if entries.is_empty() {
                    lines.push(Line::from("No saved task activity matches this query"));
                }
                for entry in entries {
                    lines.push(Line::styled(
                        format!(
                            "r{} · task #{} · {}",
                            entry.revision,
                            entry.task,
                            safe(&entry.summary)
                        ),
                        Style::default().fg(Color::Cyan),
                    ));
                    lines.extend(
                        safe(&entry.detail)
                            .lines()
                            .map(|line| Line::from(line.to_owned())),
                    );
                    lines.push(Line::from(format!("Receipt: {}", safe(&entry.correlation))));
                    lines.push(Line::default());
                }
            } else {
                lines.push(Line::from("Waiting for acknowledged task state"));
            }
            task_panel(
                frame,
                panes[1],
                tasks,
                lines,
                format!(" Saved task activity · {} ", safe(query)),
                false,
            );
        } else if let Some(report) = &tasks.autopilot_report {
            task_panel(
                frame,
                panes[1],
                tasks,
                safe(report)
                    .lines()
                    .map(|s| Line::from(s.to_owned()))
                    .collect(),
                " Autopilot · /tasks returns to task details ".into(),
                false,
            );
        } else if let Some(scope) = &tasks.scope_view {
            task_panel(
                frame,
                panes[1],
                tasks,
                safe(&scope.text())
                    .lines()
                    .map(|s| Line::from(s.to_owned()))
                    .collect(),
                "Shared Understanding · project scope".into(),
                false,
            );
        } else {
            let tree = work_tree.as_ref().unwrap();
            let focused = tasks
                .focused_work_node()
                .and_then(|id| tree.rows.iter().find(|row| row.id == id));
            let (lines, title, live, pinned) = work_inspector(tasks, tree, focused);
            task_panel_pinned(frame, panes[1], tasks, lines, title, live, pinned);
        }
    }

    if app.models_visible {
        frame.render_widget(Clear, panes[1]);
        let mut lines = vec![
            Line::from(safe(&app.models_notice)),
            Line::from("Workers: /assign ID MODEL · fresh approval required"),
            Line::default(),
        ];
        lines.extend(app.models.iter().map(|name| {
            Line::from(format!(
                "{} {}",
                if *name == session.model { "›" } else { " " },
                safe(name)
            ))
        }));
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.models_scroll, 0))
                .block(frame_block(
                    panes[1],
                    " Models · /model NAME · Esc close ".into(),
                )),
            panes[1],
        );
    }
    if let Some(completion) = &app.completion {
        let items: Vec<_> = completion
            .choices
            .iter()
            .map(|choice| ListItem::new(format!("{}  {}", choice.name, choice.description)))
            .collect();
        frame.render_widget(ratatui::widgets::Clear, panes[1]);
        frame.render_stateful_widget(
            List::new(items)
                .block(frame_block(
                    panes[1],
                    " Complete · ↑↓ choose · Enter fills draft · Esc close ".into(),
                ))
                .highlight_symbol("› ")
                .highlight_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            panes[1],
            &mut ListState::default().with_selected(Some(completion.selected)),
        );
    }
}

fn draw_transcript(
    frame: &mut Frame,
    app: &App,
    tasks: Option<&crate::task_control::TaskControl>,
    area: Rect,
) {
    let session = &app.sessions[app.selected];
    let identity = tasks.and_then(|tasks| tasks.snapshot.as_ref());
    let mut lines = Vec::new();
    let mut blocks = Vec::new();
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
        lines.push(Line::from(
            "Or chat with your local model · F1 help · F2 dashboard.",
        ));
        lines.push(Line::styled("Ctrl+N opens another concurrent chat.", dim()));
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
            let start = lines.len();
            let key = if receipt_first {
                let reference = receipts.next().unwrap();
                lines.extend(task_receipt_lines(reference, identity));
                crate::reading::BlockKey::TaskReceipt(reference.revision)
            } else {
                let command = commands.next().unwrap();
                lines.extend(command_lines(command, tasks));
                crate::reading::BlockKey::Command(command.sequence)
            };
            blocks.push(crate::reading::Block {
                key,
                start,
                len: lines.len() - start,
            });
        }
        let Some(message) = session.messages.get(index) else {
            break;
        };
        let start = lines.len();
        let (heading, color) = if message.role == "user" {
            ("You".into(), Color::Cyan)
        } else {
            response_heading(
                session.source(index),
                tasks.and_then(|tasks| tasks.canonical_scope.as_ref()),
                wayfinder_requests.get(&index.saturating_sub(1)).copied(),
            )
        };
        lines.push(Line::styled(
            heading,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        for line in safe(&message.content).lines() {
            lines.push(Line::from(line.to_owned()));
        }
        lines.push(Line::default());
        blocks.push(crate::reading::Block {
            key: crate::reading::BlockKey::Message(index),
            start,
            len: lines.len() - start,
        });
    }
    // A failed request keeps its reason in the transcript until the next attempt.
    if let Status::Failed(error) = &session.status {
        lines.push(Line::styled(
            format!("✗ Request failed · {} · Ctrl+R retries", single_line(error)),
            Style::default().fg(Color::Red),
        ));
    }
    let block = frame_block(
        area,
        format!(" Chat {} · {} ", app.selected + 1, session.short_status()),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let metadata = timing_line(session)
        .map(|text| vec![Line::styled(text, dim())])
        .unwrap_or_default();
    let metadata = Paragraph::new(metadata).wrap(Wrap { trim: false });
    let metadata_height = metadata
        .line_count(inner.width)
        .min(inner.height.saturating_sub(1) as usize) as u16;
    let content =
        Layout::vertical([Constraint::Length(metadata_height), Constraint::Min(1)]).split(inner);
    frame.render_widget(metadata, content[0]);
    if content[1].width > 0 && content[1].height > 0 {
        let heights: Vec<_> = lines
            .iter()
            .map(|line| {
                if line.width() <= usize::from(content[1].width) {
                    1
                } else {
                    Paragraph::new(line.clone())
                        .wrap(Wrap { trim: false })
                        .line_count(content[1].width)
                        .max(1)
                }
            })
            .collect();
        let position = session.reading_position_blocks(&heights, content[1].height, &blocks);
        // Slice logical lines before the widget's u16 scroll, keeping long bounded
        // responses navigable beyond 65,535 rendered rows.
        let transcript = Paragraph::new(lines.into_iter().skip(position.line).collect::<Vec<_>>())
            .wrap(Wrap { trim: false });
        frame.render_widget(
            transcript.scroll((position.row.min(u16::MAX as usize) as u16, 0)),
            content[1],
        );
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
    } else if let Some(phase) = session.wait_phase() {
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

/// One line per row: groups show their name and size; tasks show glyph, id and title.
fn work_row(
    row: &crate::mission_work::Row,
    snapshot: Option<&crate::tasks::Snapshot>,
    width: usize,
) -> ListItem<'static> {
    use crate::mission_work::NodeId;
    let indent = "  ".repeat(row.depth.min(3));
    let Some(id) = row.task else {
        let marker = if row.expanded { "▾" } else { "▸" };
        let name = match row.id {
            // The plan's receipt revision stays in F4; show its request instead.
            NodeId::Plan(_) => row
                .label
                .split_once(" · ")
                .map_or(row.label.as_str(), |(_, prompt)| prompt)
                .to_owned(),
            _ => "Manual tasks".into(),
        };
        let count = format!(" · {}", row.task_count);
        let name = truncate(
            &single_line(&name),
            width.saturating_sub(indent.width() + 2 + count.width()),
        );
        return ListItem::new(Line::styled(
            format!("{indent}{marker} {name}{count}"),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    };
    let task = snapshot.and_then(|state| state.tasks.iter().find(|task| task.id == id));
    let (glyph, color) = match (snapshot, task) {
        (Some(snapshot), Some(task)) => dashboard::glyph(snapshot, task),
        _ => ("?", Color::Gray),
    };
    let repair = if matches!(row.parent, Some(NodeId::Task(_))) {
        "↳ "
    } else {
        ""
    };
    let prefix = format!("{indent}{repair}");
    let head = format!("{glyph} #{id} ");
    let title = truncate(
        &single_line(&row.label),
        width.saturating_sub(prefix.width() + head.width()),
    );
    ListItem::new(Line::from(vec![
        Span::raw(prefix),
        Span::styled(head, Style::default().fg(color)),
        Span::raw(title),
    ]))
}

fn dependency_ids(ids: &[u64]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn section(text: &str) -> Line<'static> {
    Line::styled(
        format!("── {text} ──"),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

/// Selected task detail: status and next action first; live output while a
/// worker runs, otherwise the verified outcome, diff and check result.
/// Receipt identifiers and revisions stay in the F3/F4 views.
fn work_inspector(
    tasks: &crate::task_control::TaskControl,
    tree: &crate::mission_work::Tree,
    focused: Option<&crate::mission_work::Row>,
) -> (Vec<Line<'static>>, String, bool, usize) {
    use crate::tasks::TaskStatus;
    let Some(snapshot) = tasks.snapshot.as_ref() else {
        return (
            vec![Line::from("Task state unavailable; /refresh to retry")],
            " Mission Work · state unavailable ".into(),
            false,
            0,
        );
    };
    let counts = format!(
        "Showing {} / {} tasks · {} local workers · {} start errors",
        tree.matched_tasks,
        tree.total_tasks,
        tasks.workers.len(),
        tasks.dispatch.failures.len()
    );
    let filter = format!(
        "Filter: {} · /tasks QUERY or #ID",
        if tasks.task_query.is_empty() {
            "all".into()
        } else {
            single_line(&tasks.task_query)
        }
    );
    let Some(row) = focused else {
        return (
            vec![
                Line::from(if snapshot.tasks.is_empty() {
                    "No tasks proposed · /go GOAL or /task to propose work"
                } else if tree.rows.is_empty() {
                    "No matching tasks · /tasks clears the filter"
                } else {
                    "Select a task or group with ↑↓"
                }),
                Line::from(counts),
                Line::from(filter),
            ],
            " Mission Work ".into(),
            false,
            0,
        );
    };
    let Some(task) = row
        .task
        .and_then(|id| snapshot.tasks.iter().find(|task| task.id == id))
    else {
        let name = row
            .label
            .split_once(" · ")
            .filter(|_| matches!(row.id, crate::mission_work::NodeId::Plan(_)))
            .map_or(row.label.as_str(), |(_, prompt)| prompt);
        return (
            vec![
                Line::styled(
                    format!("Work group · {}", single_line(name)),
                    Style::default().fg(Color::Cyan),
                ),
                Line::from(format!(
                    "{} tasks in this group, including repair descendants",
                    row.task_count
                )),
                Line::from("Select a task with ↑↓ to inspect evidence or use task actions."),
                Line::from("This group has no task action target."),
                Line::from("Alt+← collapses · Alt+→ expands · PgUp/PgDn scrolls details"),
                Line::from(counts),
                Line::from(filter),
            ],
            " Work group ".into(),
            false,
            0,
        );
    };
    let status = match task.status {
        TaskStatus::Proposed => "Needs approval",
        TaskStatus::Approved => "Approved · /run after explicit policy",
        TaskStatus::Cancelled => "Cancelled",
        TaskStatus::Running => "Running",
        TaskStatus::NeedsHumanReview => "Held for human review · /review to resolve",
        TaskStatus::ReviewReady => "Check passed · needs review",
        TaskStatus::Accepted
            if snapshot.decision_for_task(task.id).is_some_and(|decision| {
                decision.outcome == crate::assessment::Outcome::ApprovedWithLimitations
            }) =>
        {
            "Accepted with limitations · inspect recorded review"
        }
        TaskStatus::Rejected
            if snapshot.decision_for_task(task.id).is_some_and(|decision| {
                decision.outcome == crate::assessment::Outcome::NeedsRepair
            }) =>
        {
            "Needs repair · inspect linked repair or propose one with /repair"
        }
        TaskStatus::Accepted => "Accepted · changes retained in worktree",
        TaskStatus::Failed => "Failed · inspect retained evidence",
        TaskStatus::Rejected => "Rejected · propose a repair task",
    };
    let (glyph, color) = dashboard::glyph(snapshot, task);
    let live = tasks.worker_live(task.id);
    let mut lines = vec![
        Line::styled(
            format!("{glyph} #{} · {status}", task.id),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Line::from(safe(&task.title)),
        Line::styled(format!("Model: {}", single_line(&task.model)), dim()),
    ];
    if !row.detail.is_empty() {
        lines.push(Line::styled(
            safe(&row.detail),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(parent) = task.repair_of {
        lines.push(Line::from(format!("Repair of #{parent}")));
    }
    if !task.dependencies.is_empty() {
        lines.push(Line::from(format!(
            "Depends on {}",
            dependency_ids(&task.dependencies)
        )));
    }
    if let Some(error) = tasks.dispatch.failures.get(&task.id) {
        lines.push(Line::styled(
            format!(
                "Start paused: {} · /run {} retries explicitly",
                safe(error),
                task.id
            ),
            Style::default().fg(Color::Red),
        ));
    }
    let actions = match task.status {
        TaskStatus::Proposed => format!(
            "/permit {} JSON · /approve {} · /cancel-task {}",
            task.id, task.id, task.id
        ),
        TaskStatus::Approved => format!("/run {} · /cancel-task {}", task.id, task.id),
        TaskStatus::Running => format!("/cancel-task {}", task.id),
        TaskStatus::ReviewReady => format!(
            "/evidence {} · /accept {} · /reject {}",
            task.id, task.id, task.id
        ),
        TaskStatus::NeedsHumanReview => format!("/review {} JSON", task.id),
        TaskStatus::Rejected | TaskStatus::Failed if snapshot.architecture_required(task.id) => {
            format!("/architect-revise {}", task.id)
        }
        TaskStatus::Rejected | TaskStatus::Failed
            if snapshot.resolved_by(task.id).is_some()
                || snapshot.architecture_obsolete(task.id) =>
        {
            format!("/evidence {}", task.id)
        }
        TaskStatus::Rejected | TaskStatus::Failed => snapshot
            .tasks
            .iter()
            .rev()
            .find(|child| child.repair_of == Some(task.id))
            .map(|child| format!("/tasks #{} to inspect repair", child.id))
            .unwrap_or_else(|| format!("/evidence {} · /repair {} REASON", task.id, task.id)),
        TaskStatus::Accepted
            if task.repair_of.is_some() && snapshot.resolution_for_family(task.id).is_none() =>
        {
            format!("/resolve-repair {}", task.id)
        }
        TaskStatus::Accepted => format!("/evidence {}", task.id),
        TaskStatus::Cancelled => "No execution action · inspect retained history".into(),
    };
    lines.push(Line::styled(format!("Task actions · {actions}"), dim()));
    lines.push(Line::default());
    let is_live = live.is_some();
    let mut live_parts = None;
    if let Some(live) = live {
        let stage = Line::styled(
            format!(
                "{}Live · {}",
                if live.cancelling {
                    "Cancellation requested · "
                } else {
                    ""
                },
                safe(&live.stage)
            ),
            Style::default().fg(Color::Yellow),
        );
        let mut output_lines = Vec::new();
        for (label, output) in [
            ("Model output", &live.model_output),
            ("Check stdout", &live.stdout),
            ("Check stderr", &live.stderr),
        ] {
            if output.trim().is_empty() {
                continue;
            }
            output_lines.push(section(label));
            let text = safe(output);
            let all: Vec<&str> = text.lines().collect();
            // The panel follows the tail; keep a bounded window of recent lines.
            output_lines.extend(
                all[all.len().saturating_sub(400)..]
                    .iter()
                    .map(|line| Line::from((*line).to_owned())),
            );
        }
        live_parts = Some((stage, output_lines));
    } else if task.status == TaskStatus::Running {
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
        lines.push(Line::styled(
            observation,
            Style::default().fg(Color::Yellow),
        ));
    } else if let Some(run) = &task.run {
        match tasks.outcome(task).as_deref() {
            Some(Ok(outcome)) => lines.extend(outcome.iter().cloned()),
            _ => lines.push(Line::from(format!(
                "Last run · {}",
                single_line(&run.detail)
            ))),
        }
    }
    if !is_live {
        lines.push(Line::default());
        lines.push(Line::styled(
            if task
                .run
                .as_ref()
                .is_some_and(|run| run.evidence_sha256.is_some())
            {
                format!(
                    "Evidence recorded for this run · F3 or /evidence {} verifies and opens it",
                    task.id
                )
            } else {
                "Evidence: no completed run evidence recorded".into()
            },
            dim(),
        ));
    }
    if let Some(policy) = &task.policy {
        lines.push(Line::styled(
            safe(&format!(
                "Files: {} · check: {}",
                policy.files.join(", "),
                policy.check.join(" ")
            )),
            dim(),
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
        lines.push(Line::from(format!("Review branch: {}", safe(name))));
    }
    if let Some(summary) = snapshot.review_summary_for_task(task.id) {
        lines.extend(summary.lines().map(|line| Line::from(safe(line))));
    }
    let criteria = snapshot.acceptance_for_task(task.id);
    if criteria.is_empty() {
        lines.push(Line::styled("Acceptance criteria: not recorded", dim()));
    } else {
        lines.push(Line::from(
            "Acceptance criteria · review each before accepting",
        ));
        lines.extend(
            criteria
                .iter()
                .enumerate()
                .map(|(index, item)| Line::from(format!("{}. {}", index + 1, safe(item)))),
        );
    }
    // Live: status, title and stage stay pinned above the streaming output tail.
    let pinned = match live_parts {
        Some((stage, output)) => {
            lines.insert(2, stage);
            lines.extend(output);
            3
        }
        None => 0,
    };
    (
        lines,
        format!(" Task #{} · {} ", task.id, dashboard::state_word(task)),
        is_live,
        pinned,
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
            let first = parts.next().unwrap_or_default().to_owned();
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
