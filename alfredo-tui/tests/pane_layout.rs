//! Side pane, header, detail and footer layout: TestBackend buffers.
use alfredo_tui::{
    autopilot::{RunState, Status as AutopilotStatus},
    conversations::TaskView,
    health::{Health, HealthView},
    mission_work::NodeId,
    model::{App, Update},
    planner::{Plan, Step},
    side_pane::{MissionEntry, MissionProgress, Section},
    task_control::TaskControl,
    tasks::{Action, Receipt, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore, WorkPolicy},
    theme::{ColorMode, BRAILLE},
    ui,
    worker::Progress,
};
use ratatui::{backend::TestBackend, buffer::Buffer, style::Color, Terminal};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use unicode_width::UnicodeWidthStr;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    control: TaskControl,
    app: App,
    _senders: Vec<tokio::sync::watch::Sender<Progress>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn render(&self, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &self.app, &self.control))
            .unwrap();
        terminal.backend().buffer().clone()
    }
    fn select(&mut self, task: u64) {
        self.control
            .restore_view(TaskView {
                visible: true,
                selected: Some(task),
                query: String::new(),
            })
            .unwrap();
    }
}

fn task(id: u64, title: &str, status: TaskStatus, repair_of: Option<u64>) -> Task {
    Task {
        id,
        title: title.into(),
        model: "qwen2.5-coder:14b".into(),
        dependencies: if id == 2 { vec![1] } else { vec![] },
        status: status.clone(),
        policy: Some(WorkPolicy {
            files: vec!["textutil.py".into()],
            check: vec!["python3".into(), "-c".into(), "import textutil".into()],
        }),
        repair_of,
        run: matches!(status, TaskStatus::Running | TaskStatus::Failed).then(|| TaskRun {
            id: format!("task-{id}-run-1"),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: None,
            detail: "Recorded".into(),
        }),
    }
}

const GOAL: &str = "Plan exactly two tasks. Task 1: create textutil.py with word_count(s) and reverse_words(s). Task 2 (depends on task 1): create test_textutil.py with unittest tests for both.";

fn fixture(tasks: Vec<Task>) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "alfredo-pane-layout-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("Danish-Immigration-Assistant");
    fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "default").unwrap();
    let mut control = TaskControl::new(store);
    let steps = tasks
        .iter()
        .filter(|task| task.repair_of.is_none())
        .map(|task| Step {
            title: task.title.clone(),
            acceptance: vec!["Works".into()],
            model: task.model.clone(),
            dependencies: vec![],
            policy: task.policy.clone().unwrap(),
        })
        .collect();
    control.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "default".into(),
        revision: 1,
        tasks,
        receipts: vec![Receipt {
            task: 1,
            revision: 1,
            request: Request {
                correlation: "plan".into(),
                expected_revision: 0,
                action: Action::Plan {
                    plan: Plan {
                        prompt: format!("{GOAL} | The previous plan was rejected by validation: Task 1 check must be an argv array. Return a corrected complete plan."),
                        planner: "qwen2.5-coder:14b".into(),
                        tasks: steps,
                        context: None,
                        scope: None,
                        architecture: None,
                    },
                },
            },
        }],
    });
    control.visible = true;
    let mut app = App::new("qwen2.5-coder:14b".into());
    app.health = HealthView::observed(Health::Ready {
        model: "qwen2.5-coder:14b".into(),
    });
    app.pane.missions = vec![MissionEntry {
        name: "docs-cleanup".into(),
        progress: MissionProgress::Idle,
    }];
    Fixture {
        root,
        control,
        app,
        _senders: vec![],
    }
}

fn textutil() -> Fixture {
    let mut fixture = fixture(vec![
        task(1, "Create textutil", TaskStatus::Accepted, None),
        task(2, "Create tests", TaskStatus::Running, None),
        task(3, "Repair of #2", TaskStatus::Failed, Some(2)),
    ]);
    let (sender, receiver) = tokio::sync::watch::channel(Progress {
        stage: "Running approved check",
        started: Instant::now() - Duration::from_secs(9),
        ..Default::default()
    });
    fixture
        .control
        .attach_progress(2, receiver, Arc::new(AtomicBool::new(false)));
    fixture._senders.push(sender);
    fixture.select(1);
    fixture
}

fn rows(buffer: &Buffer) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn text(buffer: &Buffer) -> String {
    rows(buffer).join("\n")
}

fn region(buffer: &Buffer, x: u16, y: u16, width: u16, height: u16) -> String {
    (y..y + height)
        .map(|row| {
            (x..x + width)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Column of the side pane's right border on the first body row.
fn pane_width(buffer: &Buffer, y: u16) -> u16 {
    (1..buffer.area.width)
        .find(|&x| buffer[(x, y)].symbol() == "┐")
        .map(|x| x + 1)
        .unwrap()
}

/// Closed boxes; a `├…┤` section separator counts as edge.
fn assert_borders_closed(buffer: &Buffer) {
    let at = |x: u16, y: u16| buffer[(x, y)].symbol().to_string();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if at(x, y) != "┌" {
                continue;
            }
            let right = (x + 1..buffer.area.width)
                .find(|&c| at(c, y) == "┐")
                .unwrap_or_else(|| panic!("open top {x},{y}:\n{}", text(buffer)));
            let bottom = (y + 1..buffer.area.height)
                .find(|&r| at(x, r) == "└")
                .unwrap_or_else(|| panic!("open left {x},{y}:\n{}", text(buffer)));
            assert_eq!(at(right, bottom), "┘", "{}", text(buffer));
            for r in y + 1..bottom {
                assert!(matches!(at(x, r).as_str(), "│" | "├"), "{}", text(buffer));
                assert!(
                    matches!(at(right, r).as_str(), "│" | "┤"),
                    "{}",
                    text(buffer)
                );
            }
        }
    }
}

#[test]
fn wide_layout_has_a_missions_and_work_pane_sized_to_the_terminal() {
    let fixture = textutil();
    for (width, expected) in [(88, 28), (100, 28), (140, 35), (200, 44)] {
        let buffer = fixture.render(width, 40);
        assert_borders_closed(&buffer);
        assert_eq!(
            pane_width(&buffer, 1),
            expected,
            "{width}\n{}",
            text(&buffer)
        );
        let pane = region(&buffer, 0, 1, expected, 30);
        let lines: Vec<&str> = pane.lines().collect();
        assert!(lines[0].starts_with("┌ missions "), "{pane}");
        assert!(lines[1].starts_with("│ ● default "), "{pane}");
        assert!(lines[1].trim_end().ends_with("1/2 │"), "{pane}");
        assert!(lines[2].starts_with("│ · docs-cleanup "), "{pane}");
        assert!(lines[2].contains("idle"), "{pane}");
        // One blank row between sections, then the work section title.
        assert_eq!(lines[3].trim_matches(['│', ' ']), "", "{pane}");
        assert!(lines[4].starts_with("├ work  1/2 done "), "{pane}");
        assert!(lines[4].ends_with('┤'), "{pane}");
        assert!(lines[5].starts_with("│ ▾ Plan exactly two ta"), "{pane}");
        assert!(lines[5].trim_end().ends_with(" 3 │"), "{pane}");
        assert!(!pane.contains("previous plan was rejected"), "{pane}");
        assert!(lines[6].starts_with("│   ▤ ✓ #1 Create textutil"), "{pane}");
        let running = lines[7];
        assert!(running.starts_with("│   ▤ "), "{pane}");
        let spinner = running.chars().nth(6).unwrap().to_string();
        assert!(BRAILLE.contains(&spinner.as_str()), "{pane}");
        assert!(running.contains("#2 Create tests"), "{pane}");
        let second = lines[8];
        assert!(second.contains("check"), "{pane}");
        assert!(second.contains("0:09"), "{pane}");
        assert!(lines[9].starts_with("│     ⑂ ✗ #3 Repair of #2"), "{pane}");
        assert!(lines[10].starts_with("│ ◈ ○ chat 1"), "{pane}");
        assert!(lines[10].trim_end().ends_with("ready │"), "{pane}");
    }
}

#[test]
fn running_second_line_is_dim_and_uses_the_short_model_when_narrow() {
    let fixture = textutil();
    let buffer = fixture.render(100, 30);
    let pane = region(&buffer, 0, 1, 28, 20);
    let line = pane.lines().find(|line| line.contains("0:09")).unwrap();
    assert!(line.contains("check  qwen"), "{pane}");
    assert!(!line.contains(":14b"), "{pane}");
    let y = 1 + pane.lines().position(|l| l.contains("0:09")).unwrap() as u16;
    let x = line.chars().position(|c| c == 'c').unwrap() as u16;
    assert_eq!(buffer[(x, y)].fg, Color::DarkGray);
    let medium = text(&fixture.render(150, 40));
    assert!(medium.contains("check  qwen2.5-coder  0:09"), "{medium}");
    let wide = fixture.render(200, 40);
    assert!(
        text(&wide).contains("check  qwen2.5-coder:14b  0:09"),
        "{}",
        text(&wide)
    );
}

#[test]
fn header_is_two_short_rows_without_zero_value_noise() {
    let mut fixture = textutil();
    fixture.control.autopilot = Some(AutopilotStatus {
        state: RunState::Done,
        goal: GOAL.into(),
        done: 1,
        total: 1,
        failed: 0,
        repairs: 0,
        elapsed: Duration::from_secs(62),
        branch: Some("alfredo/go-1cdb9a81".into()),
    });
    for width in [88, 100, 140, 200] {
        let lines = rows(&fixture.render(width, 30));
        let first = lines[0].trim_end();
        assert!(
            first.starts_with(" ALFREDO  default · Danish-Immigration-Assistant"),
            "{first}"
        );
        assert!(first.ends_with("warm"), "{first}");
        for noise in ["dispatch", "Work 0", "no pending review", "Mission:", "/"] {
            assert!(!first.contains(noise), "{noise}: {first}");
        }
        assert_eq!(
            lines[1].trim_end(),
            " Autopilot ✓ done   1/1   01:02   alfredo/go-1cdb9a81"
        );
        let screen = lines.join("\n");
        // The goal appears once: as the group title.
        assert_eq!(screen.matches("Plan exactly two").count(), 1, "{screen}");
    }
    // Attention items appear only when non-zero.
    fixture.control.snapshot.as_mut().unwrap().tasks[0].status = TaskStatus::ReviewReady;
    fixture.control.dispatch.enabled = true;
    let first = rows(&fixture.render(140, 30))[0].clone();
    assert!(first.contains("   1 review   dispatch on"), "{first}");
    // Failures and repairs show only when present; no autopilot, no second row.
    fixture.control.autopilot.as_mut().unwrap().failed = 1;
    assert!(rows(&fixture.render(140, 30))[1].contains("   1 failed"));
    fixture.control.autopilot = None;
    let lines = rows(&fixture.render(140, 30));
    assert!(lines[1].starts_with("┌ missions"), "{}", lines[1]);
}

#[test]
fn task_detail_is_labeled_sections_without_instructions() {
    let mut fixture = textutil();
    fixture.select(1);
    let buffer = fixture.render(140, 40);
    let detail = region(&buffer, 35, 2, 105, 30);
    let lines: Vec<&str> = detail.lines().map(str::trim_end).collect();
    assert!(lines[0].starts_with("│ ✓ #1  Create textutil"), "{detail}");
    assert!(
        lines[1].starts_with("│ Accepted · qwen2.5-coder:14b"),
        "{detail}"
    );
    assert_eq!(lines[2].trim_matches(['│', ' ']), "", "{detail}");
    let files = lines
        .iter()
        .position(|l| l.contains("Files"))
        .expect(&detail);
    assert!(
        lines[files].starts_with("│ Files    textutil.py"),
        "{detail}"
    );
    assert!(
        lines[files + 1].starts_with("│ Check    python3 -c import textutil"),
        "{detail}"
    );
    for noise in [
        "Task actions",
        "Alt+←",
        "Select a task",
        "Showing ",
        "Filter:",
        "Evidence recorded",
        "F3 or",
    ] {
        assert!(!detail.contains(noise), "{noise}: {detail}");
    }
    // Labels are dim, values normal.
    let y = 2 + files as u16;
    assert_eq!(buffer[(38, y)].fg, Color::DarkGray);
    assert_ne!(buffer[(47, y)].fg, Color::DarkGray);
    fixture.select(2);
    let detail = region(&fixture.render(140, 40), 35, 2, 105, 30);
    assert!(detail.contains("Depends  #1"), "{detail}");
    assert!(detail.contains("Stage    check"), "{detail}");
}

#[test]
fn group_detail_is_goal_progress_and_tasks_only() {
    let mut fixture = textutil();
    fixture.control.focus_node(NodeId::Plan(1));
    let buffer = fixture.render(100, 40);
    let detail = region(&buffer, 28, 2, 72, 30);
    assert!(
        detail.contains("Plan exactly two tasks. Task 1: create"),
        "{detail}"
    );
    assert!(!detail.contains("rejected by validation"), "{detail}");
    assert!(detail.contains("1/2 done"), "{detail}");
    assert!(detail.contains("✓ #1 Create textutil"), "{detail}");
    assert!(detail.contains("#3 Repair of #2"), "{detail}");
    for noise in [
        "Work group",
        "Select a task",
        "no task action",
        "Alt+",
        "Showing",
        "Filter",
    ] {
        assert!(!detail.contains(noise), "{noise}: {detail}");
    }
    // The goal wraps at word boundaries inside the pane.
    for line in detail.lines() {
        assert!(!line.contains("word_cou\n"), "{detail}");
    }
    fixture.control.task_query = "textutil".into();
    let filtered = text(&fixture.render(100, 40));
    assert!(filtered.contains("Filter"), "{filtered}");
}

#[test]
fn long_values_wrap_with_a_hanging_indent_under_their_label() {
    let mut fixture = textutil();
    fixture.control.snapshot.as_mut().unwrap().tasks[0]
        .policy
        .as_mut()
        .unwrap()
        .files = (1..=12).map(|n| format!("module_{n}.py")).collect();
    fixture.select(1);
    let detail = region(&fixture.render(100, 40), 28, 2, 72, 30);
    let lines: Vec<&str> = detail.lines().collect();
    let files = lines
        .iter()
        .position(|l| l.contains("│ Files    "))
        .expect(&detail);
    assert!(
        lines[files + 1].starts_with("│          module_"),
        "{detail}"
    );
    assert!(
        !lines[files]
            .trim_end_matches([' ', '│'])
            .ends_with("module_"),
        "{detail}"
    );
}

#[test]
fn narrow_layout_collapses_to_a_summary_row_with_an_f6_overlay() {
    let mut fixture = textutil();
    for (width, height) in [(80, 24), (87, 30), (40, 12)] {
        let buffer = fixture.render(width, height);
        assert_borders_closed(&buffer);
        let lines = rows(&buffer);
        assert!(!text(&buffer).contains("missions"), "{}", text(&buffer));
        let summary = &lines[1];
        assert!(summary.contains("1/2 done"), "{summary}");
        assert!(summary.contains("F6"), "{summary}");
    }
    let summary = rows(&fixture.render(80, 24))[1].clone();
    assert!(summary.contains("default"), "{summary}");
    assert!(summary.contains("1 working"), "{summary}");
    fixture.app.pane.toggle_focus(true);
    for (width, height) in [(80, 24), (40, 12)] {
        let buffer = fixture.render(width, height);
        assert_borders_closed(&buffer);
        let screen = text(&buffer);
        assert!(screen.contains("missions"), "{screen}");
        assert!(screen.contains("#1 Create textutil"), "{screen}");
    }
}

#[test]
fn f6_focus_highlights_the_section_title_and_footer_hints_follow_focus() {
    let mut fixture = textutil();
    let footer = |buffer: &Buffer| rows(buffer)[usize::from(buffer.area.height) - 2].clone();
    let buffer = fixture.render(140, 40);
    let hints = footer(&buffer);
    assert!(hints.contains("F6 pane"), "{hints}");
    assert!(hints.contains("F2 chat"), "{hints}");
    fixture.app.pane.toggle_focus(false);
    let buffer = fixture.render(140, 40);
    let hints = footer(&buffer);
    for hint in ["↑↓ move", "Tab section", "Enter open", "Esc prompt"] {
        assert!(hints.contains(hint), "{hints}");
    }
    let lines = rows(&buffer);
    let y = lines.iter().position(|l| l.starts_with("├ work")).unwrap() as u16;
    assert_eq!(buffer[(2, y)].fg, Color::Cyan);
    assert_eq!(buffer[(2, 1)].fg, Color::DarkGray);
    fixture.app.pane.focus = Some(Section::Missions);
    let buffer = fixture.render(140, 40);
    assert_eq!(buffer[(2, 1)].fg, Color::Cyan);
    for width in [40u16, 80, 100, 140, 200] {
        fixture.app.pane.focus = None;
        for visible in [true, false] {
            fixture.control.visible = visible;
            let hints = footer(&fixture.render(width, 24));
            let trimmed = hints.trim();
            assert!(trimmed.width() <= usize::from(width), "{hints}");
            assert!(trimmed.split("  ").count() <= 8, "{hints}");
            assert!(!trimmed.contains(" · "), "{hints}");
            assert!(
                trimmed.ends_with("quit") || trimmed.ends_with("help"),
                "{hints}"
            );
        }
    }
}

#[test]
fn chat_turns_are_separated_with_dim_speaker_labels() {
    let mut fixture = textutil();
    fixture.control.set_visible(false);
    let session = &mut fixture.app.sessions[0];
    session.insert("hello");
    session.begin().unwrap();
    session.apply(1, Update::Token("hi there".into()));
    session.apply(1, Update::Done);
    session.insert("again");
    session.begin().unwrap();
    session.apply(2, Update::Token("second answer".into()));
    session.apply(2, Update::Done);
    let buffer = fixture.render(140, 40);
    // The right pane starts at column 35: border, one column of padding, text.
    let chat = region(&buffer, 35, 0, 105, 40);
    let lines: Vec<&str> = chat.lines().collect();
    let you = lines
        .iter()
        .position(|l| l.starts_with("│ You"))
        .expect(&chat);
    assert!(lines[you + 1].contains("hello"), "{chat}");
    // The speaker label sits alone on the line above the answer.
    let answer = lines.iter().position(|l| l.contains("hi there")).unwrap();
    assert!(!lines[answer - 1].contains("hi there"), "{chat}");
    assert_ne!(lines[answer - 1].trim_matches(['│', ' ']), "", "{chat}");
    // One blank row between turns.
    let second = lines.iter().rposition(|l| l.starts_with("│ You")).unwrap();
    assert_eq!(lines[second - 1].trim_matches(['│', ' ']), "", "{chat}");
    assert_ne!(lines[second - 2].trim_matches(['│', ' ']), "", "{chat}");
    assert_eq!(buffer[(37, you as u16)].fg, Color::DarkGray);
    assert_eq!(buffer[(37, (answer - 1) as u16)].fg, Color::DarkGray);
}

#[test]
fn no_color_strips_colours_and_truecolor_uses_the_palette() {
    let mut fixture = textutil();
    fixture.app.pane.theme.color = ColorMode::None;
    let buffer = fixture.render(140, 40);
    for y in 0..40 {
        for x in 0..140 {
            assert_eq!(buffer[(x, y)].fg, Color::Reset, "{x},{y}");
            assert_eq!(buffer[(x, y)].bg, Color::Reset, "{x},{y}");
        }
    }
    fixture.app.pane.theme.color = ColorMode::Truecolor;
    let buffer = fixture.render(140, 40);
    let lines = rows(&buffer);
    let y = lines
        .iter()
        .position(|l| l.contains("✓ #1 Create"))
        .unwrap() as u16;
    let x = lines[y as usize].chars().position(|c| c == '✓').unwrap() as u16;
    assert_eq!(buffer[(x, y)].fg, Color::Rgb(0x75, 0xd9, 0x9b));
}

#[test]
fn pane_without_task_state_lists_chats() {
    let mut app = App::new("fixture".into());
    app.add_session();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    assert_borders_closed(&buffer);
    let screen = text(&buffer);
    assert!(screen.contains("◈ ○ chat 1"), "{screen}");
    assert!(screen.contains("◈ ○ chat 2"), "{screen}");
    assert!(!screen.contains("Sessions"), "{screen}");
}
