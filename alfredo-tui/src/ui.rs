use crate::model::{App, Status};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
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
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(identity.is_some())),
        Constraint::Min(3),
        Constraint::Length(3),
        Constraint::Length(2),
    ])
    .split(area);
    let session = &app.sessions[app.selected];
    let active = app.sessions.iter().filter(|s| s.status.active()).count();
    let health = app.health.state(&session.model);
    let health = health
        .label(&session.model, area.width >= 100)
        .map(|label| {
            let color = if health.healthy() {
                Color::Green
            } else {
                Color::Red
            };
            Span::styled(format!(" {label} "), Style::default().fg(color))
        })
        .unwrap_or_default();
    // With a mission line, health sits at its right edge so the status row keeps its width.
    let health_width = u16::try_from(health.width()).unwrap_or(u16::MAX);
    let (health, mission_health) = if identity.is_some() {
        (Span::default(), Some(health))
    } else {
        (health, None)
    };
    let used = if mission_health.is_some() {
        0
    } else {
        health_width
    };
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
            Span::raw(if let Some(tasks) = tasks {
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
                let remaining = usize::from(area.width.saturating_sub(used))
                    .saturating_sub(9 + unicode_width::UnicodeWidthStr::width(header.as_str()));
                if remaining >= 16 {
                    if tasks.visible {
                        header.push_str(" · Mission Work · ↑↓ select");
                    } else {
                        header.push_str(&format!(" · {active} chats · {}", safe(&session.model)));
                        if tasks.scope_status.label.starts_with("Wayfinder /") {
                            header.push_str(&format!(" · {}", safe(&tasks.scope_status.label)));
                        }
                    }
                }
                header
            } else {
                format!(
                    "  Conversations · {active} active · {}",
                    safe(&session.model)
                )
            }),
        ])),
        rows[0],
    );
    if let Some(snapshot) = identity {
        let health_width = health_width.min(rows[1].width);
        let line = Layout::horizontal([Constraint::Min(0), Constraint::Length(health_width)])
            .split(rows[1]);
        if let Some(health) = mission_health {
            frame.render_widget(Paragraph::new(Line::from(health)), line[1]);
        }
        frame.render_widget(
            Paragraph::new(format!(
                " Mission: {} · {}",
                safe(&snapshot.mission),
                safe(&snapshot.workspace.display().to_string())
            ))
            .style(Style::default().fg(Color::Cyan)),
            line[0],
        );
    }
    let wide = area.width >= 88;
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
    let compact_tree = !wide && rows[2].height < 7;
    let panes = Layout::default()
        .direction(if wide {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if wide {
            vec![
                Constraint::Length(if focused_review {
                    if area.width < 100 {
                        0
                    } else {
                        28
                    }
                } else if work_tree.is_some() {
                    40
                } else {
                    28
                }),
                Constraint::Min(1),
            ]
        } else {
            vec![
                Constraint::Length(if focused_review {
                    0
                } else if work_tree.is_some() {
                    if compact_tree {
                        1
                    } else {
                        (rows[2].height / 3).clamp(3, 8)
                    }
                } else {
                    3
                }),
                Constraint::Min(1),
            ]
        })
        .split(rows[2]);
    let (items, list_title, selected): (Vec<ListItem>, String, Option<usize>) =
        if let Some(tasks) = tasks.filter(|tasks| tasks.visible) {
            let tree = work_tree.as_ref().unwrap();
            let selected = tasks
                .focused_work_node()
                .and_then(|selected| tree.rows.iter().position(|row| row.id == selected));
            let items = tree
                .rows
                .iter()
                .map(|row| work_row(row, identity, compact_tree || panes[0].height < 5))
                .collect();
            (
                items,
                format!(
                    " Mission Work · {}/{} tasks ",
                    tree.matched_tasks, tree.total_tasks,
                ),
                selected,
            )
        } else {
            (
                app.sessions
                    .iter()
                    .enumerate()
                    .map(|(index, s)| ListItem::new(format!("{}  {}", index + 1, s.status_label())))
                    .collect(),
                " Sessions ".into(),
                Some(app.selected),
            )
        };
    let list = List::new(items)
        .block(if compact_tree && work_tree.is_some() {
            Block::default()
        } else {
            Block::default().borders(Borders::ALL).title(list_title)
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
    if tasks.is_none_or(|tasks| !tasks.visible) && !app.models_visible && app.completion.is_none() {
        let mut lines = Vec::new();
        let mut blocks = Vec::new();
        if session.messages.is_empty()
            && session.task_receipts().is_empty()
            && session.commands().is_empty()
        {
            lines.push(Line::from("Start a conversation with your local model."));
            lines.push(Line::from("Ctrl+N opens another concurrent session."));
            lines.push(Line::from(
                "Use /task to propose coding work, then F2 to inspect permissions.",
            ));
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
        let block = Block::bordered().title(format!(
            " Session {} · {} ",
            app.selected + 1,
            session.status_label()
        ));
        let inner = block.inner(panes[1]);
        frame.render_widget(block, panes[1]);
        let mut metadata = Vec::new();
        if let Some(observation) = session.queue_observation() {
            metadata.push(Line::from(crate::client_timing::queue_summary(
                &observation,
            )));
        }
        if let Some(timing) = &session.timing {
            metadata.push(Line::from(timing.summary(std::time::Instant::now())));
        }
        if let Some(metrics) = &session.metrics {
            metadata.push(Line::from(metrics.summary()));
        }
        let metadata = Paragraph::new(metadata).wrap(Wrap { trim: false });
        let metadata_height = metadata
            .line_count(inner.width)
            .min(inner.height.saturating_sub(1) as usize) as u16;
        let content = Layout::vertical([Constraint::Length(metadata_height), Constraint::Min(1)])
            .split(inner);
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
            let transcript =
                Paragraph::new(lines.into_iter().skip(position.line).collect::<Vec<_>>())
                    .wrap(Wrap { trim: false });
            frame.render_widget(
                transcript.scroll((position.row.min(u16::MAX as usize) as u16, 0)),
                content[1],
            );
        }
    }
    // Show the tail of the draft without slicing UTF-8 in the middle of a codepoint.
    if let Some(tasks) =
        tasks.filter(|tasks| tasks.visible && !app.models_visible && app.completion.is_none())
    {
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
            );
        } else if let Some(evidence) = &tasks.evidence {
            let block = Block::bordered().title(format!(
                " Verified run evidence · task #{} · /tasks to return ",
                evidence.task
            ));
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
            );
        } else {
            let tree = work_tree.as_ref().unwrap();
            let focused = tasks
                .focused_work_node()
                .and_then(|id| tree.rows.iter().find(|row| row.id == id));
            let (lines, title) = work_inspector(tasks, tree, focused);
            task_panel(frame, panes[1], tasks, lines, title);
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
                .block(Block::bordered().title(" Models · /model NAME · Esc close ")),
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
                .block(
                    Block::bordered()
                        .title(" Complete · ↑↓ choose · Enter fills draft · Esc close "),
                )
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
            Line::from(if tasks.is_some_and(|tasks| tasks.visible) {
                if area.width < 60 {
                    "↑↓ rows Alt+←/→ tree PgUp/Dn"
                } else {
                    "↑↓ select  Alt+←/→ collapse/expand  PgUp/PgDn details  F3 evidence  F2 chat  F4 Activity  ^Q quit"
                }
            } else {
                "F4 Activity  F1 commands  F2 tasks/chat  ^N new  Tab complete/switch  Esc cancel  ^R retry  PgUp/PgDn scroll  ^Q quit"
            }),
            Line::styled(note, Style::default().fg(Color::Yellow)),
        ]),
        rows[4],
    );
}

fn single_line(text: &str) -> String {
    safe(text).replace(['\n', '\t'], " ")
}

fn work_row(
    row: &crate::mission_work::Row,
    snapshot: Option<&crate::tasks::Snapshot>,
    compact: bool,
) -> ListItem<'static> {
    use crate::mission_work::NodeId;
    let indent = "  ".repeat(row.depth.min(if compact { 2 } else { 4 }));
    let marker = if row.expandable {
        if row.expanded {
            "▾"
        } else {
            "▸"
        }
    } else if matches!(row.parent, Some(NodeId::Task(_))) {
        "↳"
    } else {
        "·"
    };
    let Some(id) = row.task else {
        let name = match row.id {
            NodeId::Plan(revision) => format!("Plan r{revision}"),
            _ => "Manual tasks".into(),
        };
        let mut lines = vec![Line::styled(
            format!(
                "{indent}{marker} {name} · {} {}",
                row.task_count,
                if row.task_count == 1 { "task" } else { "tasks" }
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        if !compact && matches!(row.id, NodeId::Plan(_)) {
            lines.push(Line::from(format!("{indent}  {}", single_line(&row.label))));
        }
        return ListItem::new(lines);
    };
    let status = single_line(&row.status);
    let label = single_line(&row.label);
    if compact {
        return ListItem::new(format!("{indent}{marker} #{id} {status} · {label}"));
    }
    let mut lines = vec![
        Line::from(format!("{indent}{marker} #{id} {status}")),
        Line::from(format!("{indent}  {label}")),
    ];
    if let Some(task) = snapshot.and_then(|state| state.tasks.iter().find(|task| task.id == id)) {
        if !task.dependencies.is_empty() {
            lines.push(Line::styled(
                format!(
                    "{indent}  Depends on {}",
                    dependency_ids(&task.dependencies)
                ),
                Style::default().fg(Color::Yellow),
            ));
        }
    }
    ListItem::new(lines)
}

fn dependency_ids(ids: &[u64]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn work_inspector(
    tasks: &crate::task_control::TaskControl,
    tree: &crate::mission_work::Tree,
    focused: Option<&crate::mission_work::Row>,
) -> (Vec<Line<'static>>, String) {
    use crate::tasks::TaskStatus;
    let Some(snapshot) = tasks.snapshot.as_ref() else {
        return (
            vec![Line::from("Task state unavailable; /refresh to retry")],
            " Mission Work · state unavailable ".into(),
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
                    "No tasks proposed · use /task to propose work"
                } else if tree.rows.is_empty() {
                    "No matching tasks · /tasks clears the filter"
                } else {
                    "Select a task or group with ↑↓"
                }),
                Line::from(counts),
                Line::from(filter),
            ],
            " Mission Work ".into(),
        );
    };
    let Some(task) = row
        .task
        .and_then(|id| snapshot.tasks.iter().find(|task| task.id == id))
    else {
        return (
            vec![
                Line::styled(format!("Work group · {}", single_line(&row.label)), Style::default().fg(Color::Cyan)),
                Line::from(format!("{} tasks in this group, including repair descendants", row.task_count)),
                Line::from(safe(&row.detail)),
                Line::from("Select a task with ↑↓ to inspect evidence or use task actions."),
                Line::from("This group has no task action target."),
                Line::from("Alt+← collapses · Alt+→ expands · PgUp/PgDn scrolls details"),
                Line::from("Plan membership and repair ancestry define the tree; dependencies are separate edges."),
                Line::from(counts),
                Line::from(filter),
            ],
            " Work group ".into(),
        );
    };
    let status = match task.status {
        TaskStatus::Proposed => "Needs approval",
        TaskStatus::Approved => "Approved · /run after explicit policy",
        TaskStatus::Cancelled => "Cancelled",
        TaskStatus::Running => "Run claimed · effects may be in progress",
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
    let mut lines = vec![
        Line::styled(
            format!("#{} · {status}", task.id),
            Style::default().fg(Color::Cyan),
        ),
        Line::from(safe(&task.title)),
        Line::from(format!("Model: {}", single_line(&task.model))),
        Line::styled(safe(&row.detail), Style::default().fg(Color::Yellow)),
    ];
    if let Some(run) = &task.run {
        lines.push(Line::from(format!(
            "Run: {} · {}",
            single_line(&run.id),
            safe(&run.detail)
        )));
    }
    if let Some(progress) = tasks.worker_progress(task.id) {
        lines.push(Line::styled(
            safe(&progress),
            Style::default().fg(Color::Yellow),
        ));
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
    }
    if let Some(error) = tasks.dispatch.failures.get(&task.id) {
        lines.push(Line::from(format!(
            "Start paused: {} · /run {} retries explicitly",
            safe(error),
            task.id
        )));
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
    lines.push(Line::from(
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
    ));
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
    lines.push(Line::from(format!("Task actions · {actions}")));
    lines.push(Line::from(counts));
    lines.push(Line::from(filter));
    lines.push(Line::from(format!(
        "Dispatch {} · /dispatch on|off",
        if tasks.dispatch.enabled { "ON" } else { "OFF" }
    )));
    lines.push(Line::styled(
        safe(&tasks.scope_status.label),
        Style::default().fg(Color::Yellow),
    ));
    lines.push(Line::from("Recent saved activity · newest first"));
    let mut activity = crate::activity::iter_entries(snapshot, &format!("#{}", task.id))
        .take(3)
        .peekable();
    if activity.peek().is_none() {
        lines.push(Line::from("No saved task activity recorded"));
    }
    lines.extend(activity.map(|entry| {
        Line::from(format!(
            "r{} · {} · {}",
            entry.revision,
            single_line(&entry.summary),
            single_line(&entry.correlation)
        ))
    }));
    if let Some((stdout, stderr)) = tasks.worker_output(task.id) {
        for (label, output) in [("stdout", stdout), ("stderr", stderr)] {
            if !output.is_empty() {
                lines.push(Line::styled(
                    format!("Live check {label} · bounded tail · /evidence after completion"),
                    Style::default().fg(Color::Yellow),
                ));
                lines.extend(
                    safe(&output)
                        .lines()
                        .map(|line| Line::from(line.to_owned())),
                );
            }
        }
    }
    if let Some(policy) = &task.policy {
        lines.push(Line::from(safe(&format!(
            "Files: {:?} · check: {:?}",
            policy.files, policy.check
        ))));
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
        lines.push(Line::from(format!(
            "Recorded review branch: {}",
            safe(name)
        )));
    }
    if let Some(summary) = snapshot.review_summary_for_task(task.id) {
        lines.extend(summary.lines().map(|line| Line::from(safe(line))));
    }
    let criteria = snapshot.acceptance_for_task(task.id);
    if criteria.is_empty() {
        lines.push(Line::from("Acceptance criteria: not recorded"));
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
    (
        lines,
        format!(
            " Task #{} · revision {} · PgUp/PgDn ",
            task.id, snapshot.revision
        ),
    )
}

/// Slice logical lines before Ratatui's u16 scroll limit. The full row offset is
/// local UI state; drawing and navigation never mutate canonical task records.
fn task_panel(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    tasks: &crate::task_control::TaskControl,
    lines: Vec<Line<'static>>,
    title: String,
) {
    frame.render_widget(Clear, area);
    let block = if area.height < 3 {
        Block::default()
    } else {
        Block::bordered().title(title)
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    tasks.scroll_height.set(inner.height);
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
    let mut remaining = tasks.scroll.min(maximum);
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

/// A compact projection of an exact canonical receipt, never model-authored text.
/// Keep three logical lines even when unavailable so saved reading offsets remain valid.
fn task_receipt_lines(
    reference: &crate::model::TaskReceiptRef,
    snapshot: Option<&crate::tasks::Snapshot>,
) -> [Line<'static>; 3] {
    let receipt = reference
        .revision
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| snapshot?.receipts.get(index))
        .filter(|receipt| {
            receipt.revision == reference.revision
                && receipt.task == reference.task
                && receipt.request.correlation == reference.correlation
        });
    let (label, phase) = match receipt {
        Some(receipt) => (
            format!("Observed task receipt · #{}", reference.task),
            receipt_phase(receipt),
        ),
        None => (
            "Task receipt unavailable".into(),
            "Reference not verified".into(),
        ),
    };
    [
        Line::styled(
            label,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from(format!(
            "{} · revision {} · {}",
            phase,
            reference.revision,
            safe(&reference.correlation)
        )),
        Line::default(),
    ]
}

fn receipt_phase(receipt: &crate::tasks::Receipt) -> String {
    use crate::tasks::Action;
    match &receipt.request.action {
        Action::Propose { .. } => "Task proposed".into(),
        Action::Plan { .. } => "Plan proposed".into(),
        Action::Assign { .. } => "Worker assigned; approval required".into(),
        Action::Permit { .. } => "Policy set; approval required".into(),
        Action::Approve { .. } => "Task approved".into(),
        Action::Cancel { .. } => "Task cancelled".into(),
        Action::Start { .. } => "Worker run claimed".into(),
        Action::Finish { status, .. } => format!("Worker result: {status:?}"),
        Action::Repair { .. } => "Repair proposed; approval required".into(),
        Action::Branch { .. } => "Review branch recorded".into(),
        Action::ResolveRepair { .. } => "Repair resolution recorded".into(),
        Action::ReviewArchitecture { task, .. } if *task == receipt.task => {
            "Architect revision required".into()
        }
        Action::ReviewArchitecture { .. } => "Architecture repair proposed".into(),
        Action::ReviewAndRepair { task, decision } => format!(
            "Review #{task}: {}; repair proposed",
            decision.outcome.label()
        ),
        Action::Decide { decision, .. } => decision.risk.map_or_else(
            || format!("Review: {}", decision.outcome.label()),
            |risk| format!("Human review required: {}", risk.label()),
        ),
        Action::Assess { assessment, .. } => if assessment.accept {
            "Review accepted with criteria"
        } else {
            "Review rejected with criteria"
        }
        .into(),
        Action::Review { accept, .. } => if *accept {
            "Review accepted"
        } else {
            "Review rejected"
        }
        .into(),
    }
}

fn command_lines(
    command: &crate::console_command::ConsoleCommand,
    tasks: Option<&crate::task_control::TaskControl>,
) -> Vec<Line<'static>> {
    use crate::console_command::CommandState;
    let automatic = matches!(
        command.intent,
        crate::command_intent::Intent::DispatchRun { .. }
    );
    let architect = matches!(
        command.intent,
        crate::command_intent::Intent::ArchitectDraft { .. }
    );
    let wayfinder_turn = match command.intent {
        crate::command_intent::Intent::Wayfinder { user_message, .. } => Some(user_message / 2 + 1),
        _ => None,
    };
    let selection = command.intent.selection_request();
    let run = automatic || matches!(command.intent, crate::command_intent::Intent::Run { .. });
    let cancel_worker = matches!(&command.intent, crate::command_intent::Intent::Control { request }
        if matches!(request.operation, crate::control_command::Operation::CancelWorker { .. }));
    let mut lifecycle = if run || cancel_worker {
        command
            .intent
            .task_receipts(tasks.and_then(|tasks| tasks.snapshot.as_ref()))
            .into_iter()
    } else {
        Vec::new().into_iter()
    };
    let acknowledgment = if run {
        lifecycle.next()
    } else {
        command.intent.reconcile(
            tasks.and_then(|tasks| tasks.snapshot.as_ref()),
            tasks.and_then(|tasks| tasks.canonical_scope.as_ref()),
        )
    }
    .map(|acknowledgment| command_acknowledgment(acknowledgment, tasks));
    if cancel_worker {
        // The Start establishes the target; it does not acknowledge requesting cancellation.
        lifecycle.next();
    }
    let acknowledged = acknowledgment.is_some();
    let phase = acknowledgment.unwrap_or_else(|| match &command.state {
        CommandState::Pending => "Pending · saving intent".into(),
        CommandState::Submitted if command.intent.planner_request().is_some() =>
        {
            "Submitted · planner operation pending".into()
        }
        CommandState::Submitted
            if matches!(command.intent, crate::command_intent::Intent::Control { .. }) =>
        {
            "Submitted · controller operation pending".into()
        }
        CommandState::Submitted if wayfinder_turn.is_some() =>
        {
            "Submitted · awaiting scope receipt".into()
        }
        CommandState::Submitted if selection.is_some() => "Submitted · preparing selection".into(),
        CommandState::Submitted => "Submitted · awaiting acknowledgment".into(),
        CommandState::Unknown { reason } => format!("Outcome unconfirmed · {reason}"),
        CommandState::Refused { reason } => format!("Not dispatched · {reason}"),
        CommandState::Selection { outcome } => selection
            .map(|request| selection_phase(request, outcome))
            .unwrap_or_else(|| "Selection observation unavailable".into()),
        CommandState::Control { outcome } => match outcome {
            crate::control_command::Outcome::CancellationRequested => "Cancellation requested".into(),
            crate::control_command::Outcome::DispatchChanged { enabled } => format!("Dispatch {} for originating controller", if *enabled { "enabled" } else { "disabled" }),
        },
        CommandState::Planner { outcome } => match outcome {
            crate::planner_command::Outcome::Generated { tasks, .. } => {
                format!("Draft generated · {tasks} {} · saving and approval are separate", if *tasks == 1 { "step" } else { "steps" })
            }
            crate::planner_command::Outcome::Stopped if matches!(&command.intent, crate::command_intent::Intent::Planner { request } if matches!(request.operation, crate::planner_command::Operation::Cancel { generation: None, .. })) => "Draft discarded".into(),
            crate::planner_command::Outcome::Stopped => "Draft generation stopped".into(),
            crate::planner_command::Outcome::Failed { reason } => {
                if matches!(&command.intent, crate::command_intent::Intent::Planner { request } if matches!(request.operation, crate::planner_command::Operation::Cancel { .. })) {
                    format!("Planner cancellation failed · {reason}")
                } else { format!("Draft generation failed · {reason}") }
            }
        },
    });
    let mut lines = vec![Line::styled(
        if matches!(
            command.intent,
            crate::command_intent::Intent::SelectionArrival { .. }
        ) {
            format!("Workspace · arrival #{}", command.sequence)
        } else if selection.is_some() {
            format!("You · selection #{}", command.sequence)
        } else if automatic {
            format!("Dispatch · launch #{}", command.sequence)
        } else if architect {
            format!("Architect · draft #{}", command.sequence)
        } else if let Some(turn) = wayfinder_turn {
            format!("Wayfinder · scope #{} · turn {turn}", command.sequence)
        } else {
            format!("You · command #{}", command.sequence)
        },
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];
    lines.extend(
        safe(&command.text)
            .lines()
            .map(|line| Line::from(line.to_owned())),
    );
    lines.push(Line::styled(
        safe(&phase).replace(['\n', '\t'], " "),
        Style::default().fg(
            if let CommandState::Selection { outcome } = &command.state {
                if outcome.failure.is_some() {
                    Color::Red
                } else {
                    Color::White
                }
            } else if acknowledged {
                Color::Green
            } else {
                Color::Yellow
            },
        ),
    ));
    if run || cancel_worker {
        let acknowledgment = lifecycle.next();
        let color = match acknowledgment.as_ref() {
            Some(crate::command_intent::Acknowledgment::Task { revision, .. }) => tasks
                .and_then(|tasks| tasks.snapshot.as_ref())
                .and_then(|snapshot| snapshot.receipts.get(revision.saturating_sub(1) as usize))
                .map(|receipt| match &receipt.request.action {
                    crate::tasks::Action::Finish {
                        status: crate::tasks::TaskStatus::Failed,
                        ..
                    } => Color::Red,
                    crate::tasks::Action::Finish {
                        status: crate::tasks::TaskStatus::Cancelled,
                        ..
                    } => Color::Yellow,
                    _ => Color::Green,
                })
                .unwrap_or(Color::Yellow),
            _ => Color::Yellow,
        };
        let result =
            acknowledgment.map(|acknowledgment| command_acknowledgment(acknowledgment, tasks));
        lines.push(Line::styled(
            result.unwrap_or_else(|| "Result not acknowledged".into()),
            Style::default().fg(color),
        ));
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

fn command_acknowledgment(
    acknowledgment: crate::command_intent::Acknowledgment,
    tasks: Option<&crate::task_control::TaskControl>,
) -> String {
    match acknowledgment {
        crate::command_intent::Acknowledgment::Task {
            revision,
            task,
            correlation,
        } => {
            let phase = revision
                .checked_sub(1)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| tasks?.snapshot.as_ref()?.receipts.get(index))
                .filter(|receipt| {
                    receipt.revision == revision
                        && receipt.task == task
                        && receipt.request.correlation == correlation
                })
                .map(receipt_phase)
                .unwrap_or_else(|| "Task action acknowledged".into());
            format!("{phase} · Task receipt r{revision} · task #{task} · {correlation}")
        }
        crate::command_intent::Acknowledgment::Scope {
            revision,
            correlation,
        } => {
            format!("Scope receipt r{revision} · {correlation}")
        }
    }
}
