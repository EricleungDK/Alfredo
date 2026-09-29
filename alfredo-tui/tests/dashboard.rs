//! Multi-agent dashboard presentation: buffer tests at the supported sizes.
use alfredo_tui::{
    autopilot::{RunState, Status as AutopilotStatus},
    conversations::TaskView,
    health::{Health, HealthView},
    model::App,
    task_control::TaskControl,
    tasks::{Action, Request, Snapshot, Task, TaskRun, TaskStatus, TaskStore, WorkPolicy},
    ui,
    worker::{Evidence, Progress},
};
use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    root: PathBuf,
    pub store: TaskStore,
    pub control: TaskControl,
    pub app: App,
}

impl Fixture {
    pub fn new(tasks: Vec<Task>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-dashboard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = TaskStore::new(&root.join("state"), &workspace, "Dashboard").unwrap();
        let mut control = TaskControl::new(store.clone());
        control.visible = true;
        control.snapshot = Some(Snapshot {
            schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
            workspace,
            mission: "Dashboard".into(),
            revision: 0,
            tasks,
            receipts: vec![],
        });
        let app = App::new("qwen2.5-coder:14b".into());
        Self {
            root,
            store,
            control,
            app,
        }
    }

    pub fn select(&mut self, task: u64) {
        self.control
            .restore_view(TaskView {
                visible: true,
                selected: Some(task),
                query: String::new(),
            })
            .unwrap();
    }

    pub fn render(&self, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &self.app, &self.control))
            .unwrap();
        terminal.backend().buffer().clone()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn task(id: u64, title: &str, status: TaskStatus, dependencies: Vec<u64>) -> Task {
    Task {
        id,
        title: title.into(),
        model: "qwen2.5-coder:14b".into(),
        dependencies,
        status: status.clone(),
        policy: Some(WorkPolicy {
            files: vec!["calc.py".into()],
            check: vec!["python3".into(), "-m".into(), "unittest".into()],
        }),
        repair_of: None,
        run: matches!(
            status,
            TaskStatus::Running
                | TaskStatus::Failed
                | TaskStatus::Accepted
                | TaskStatus::ReviewReady
        )
        .then(|| TaskRun {
            id: format!("task-{id}-run-1"),
            baseline: "a".repeat(40),
            inputs: vec![],
            evidence_sha256: None,
            detail: "Recorded outcome".into(),
        }),
    }
}

pub fn rows(buffer: &Buffer) -> Vec<String> {
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

/// Every box corner must belong to a complete rectangle: no overlapping or clipped borders.
/// A `├…┤` section separator inside a box counts as its edge.
pub fn assert_borders_closed(buffer: &Buffer) {
    let width = buffer.area.width;
    let height = buffer.area.height;
    let at = |x: u16, y: u16| buffer[(x, y)].symbol().to_string();
    for y in 0..height {
        for x in 0..width {
            if at(x, y) != "┌" {
                continue;
            }
            let right = (x + 1..width)
                .find(|&column| at(column, y) == "┐")
                .unwrap_or_else(|| panic!("open top border at {x},{y}:\n{}", text(buffer)));
            let bottom = (y + 1..height)
                .find(|&row| at(x, row) == "└")
                .unwrap_or_else(|| panic!("open left border at {x},{y}:\n{}", text(buffer)));
            assert_eq!(at(right, bottom), "┘", "box {x},{y}:\n{}", text(buffer));
            for row in y + 1..bottom {
                let (left, edge) = (at(x, row), at(right, row));
                assert!(
                    left == "│" || (left == "├" && edge == "┤"),
                    "left edge {x},{row}:\n{}",
                    text(buffer)
                );
                assert!(
                    edge == "│" || (left == "├" && edge == "┤"),
                    "right edge {right},{row}:\n{}",
                    text(buffer)
                );
            }
        }
    }
}

fn calc_plan() -> Fixture {
    Fixture::new(vec![
        task(1, "Create calc.py", TaskStatus::Accepted, vec![]),
        task(2, "Create test_calc.py", TaskStatus::Running, vec![1]),
        task(3, "Write README", TaskStatus::Proposed, vec![]),
        task(4, "Review edge cases", TaskStatus::ReviewReady, vec![]),
        task(5, "Old attempt", TaskStatus::Failed, vec![]),
        task(6, "Package it", TaskStatus::Approved, vec![5]),
    ])
}

#[test]
fn dashboard_lists_one_line_per_task_with_status_glyphs_and_progress() {
    let mut fixture = calc_plan();
    fixture.select(1);
    for (width, height) in [(80, 24), (100, 30), (140, 40)] {
        let buffer = fixture.render(width, height);
        let screen = text(&buffer);
        assert!(screen.contains("1/6 done"), "{width}: {screen}");
        // Below 88 columns the pane is one summary row; F6 opens it as an overlay.
        let buffer = if width < 88 {
            fixture.app.pane.toggle_focus(true);
            let overlay = fixture.render(width, height);
            fixture.app.pane.toggle_focus(true);
            overlay
        } else {
            buffer
        };
        let screen = text(&buffer);
        // Long titles are cut to the pane width.
        for row in [
            "✓ #1 Create calc.py",
            "▶ #2 Create test",
            "○ #3 Write README",
            "◐ #4 Review edge",
            "✗ #5 Old attempt",
            "‖ #6 Package it",
        ] {
            assert_eq!(screen.matches(row).count(), 1, "{width}: {row}\n{screen}");
        }
        assert_borders_closed(&buffer);
    }
}

#[test]
fn running_task_shows_live_stage_model_text_and_check_output_following_the_tail() {
    let mut fixture = calc_plan();
    fixture.select(2);
    let (sender, receiver) = tokio::sync::watch::channel(Progress {
        stage: "Receiving model plan",
        model_output: (1..=60)
            .map(|line| format!("model line {line}\n"))
            .collect(),
        ..Default::default()
    });
    fixture
        .control
        .attach_progress(2, receiver, Arc::new(AtomicBool::new(false)));
    let screen = text(&fixture.render(100, 30));
    assert!(screen.contains("Receiving model plan"), "{screen}");
    assert!(
        screen.contains("model line 60"),
        "tail not followed: {screen}"
    );
    assert!(!screen.contains("model line 1\n"), "{screen}");
    // New output appears on the next redraw without any keypress.
    sender.send_modify(|progress| {
        progress.stage = "Running approved check";
        progress.check_stdout = b"Ran 2 tests in 0.001s\nOK\n".to_vec();
    });
    let screen = text(&fixture.render(100, 30));
    assert!(screen.contains("Running approved check"), "{screen}");
    assert!(screen.contains("Ran 2 tests"), "{screen}");
    // Scrolling up stops following; paging to the end resumes it.
    fixture.control.page_details(false);
    let screen = text(&fixture.render(100, 30));
    assert!(!screen.contains("Ran 2 tests"), "{screen}");
    for _ in 0..20 {
        fixture.control.page_details(true);
    }
    sender.send_modify(|progress| progress.check_stdout.extend(b"LATEST\n"));
    let screen = text(&fixture.render(100, 30));
    assert!(screen.contains("LATEST"), "{screen}");
}

#[test]
fn running_task_shows_streamed_file_blocks_as_code_without_markers() {
    let mut fixture = calc_plan();
    fixture.select(2);
    let (_sender, receiver) = tokio::sync::watch::channel(Progress {
        stage: "Receiving model plan",
        model_output: "I will add both files.\n=== FILE: todo.py ===\nimport json\nprint(f'{todo[\"task\"]}')\n=== END FILE ===\n=== FILE: test_todo.py ===\nimport unittest\nclass T(unittest.TestCase):\n".into(),
        ..Default::default()
    });
    fixture
        .control
        .attach_progress(2, receiver, Arc::new(AtomicBool::new(false)));
    let buffer = fixture.render(100, 30);
    let screen = text(&buffer);
    for shown in [
        "▸ todo.py",
        "import json",
        "print(f'{todo[\"task\"]}')",
        "▸ test_todo.py",
        "class T(unittest.TestCase):",
    ] {
        assert!(screen.contains(shown), "{shown} missing: {screen}");
    }
    for hidden in ["=== FILE", "END FILE", "\\n"] {
        assert!(!screen.contains(hidden), "{hidden} shown: {screen}");
    }
    assert_borders_closed(&buffer);
}

#[test]
fn default_task_detail_hides_receipt_ids_and_revisions() {
    let mut fixture = calc_plan();
    let snapshot = fixture.control.snapshot.as_mut().unwrap();
    snapshot.revision = 1;
    snapshot.receipts.push(alfredo_tui::tasks::Receipt {
        revision: 1,
        task: 1,
        request: Request {
            correlation: "6041-1790605051159132520-1".into(),
            expected_revision: 0,
            action: Action::Approve { task: 1 },
        },
    });
    fixture.select(1);
    for (width, height) in [(80, 24), (100, 30), (140, 40)] {
        let screen = text(&fixture.render(width, height));
        for hidden in [
            "6041-1790605051159132520-1",
            "revision 1",
            "receipt r1",
            "task-1-run-1",
        ] {
            assert!(!screen.contains(hidden), "{hidden} at {width}: {screen}");
        }
        assert!(screen.contains("Create calc.py"), "{screen}");
    }
    // The IDs stay available in the activity view.
    fixture.control.activity = Some("#1".into());
    let screen = text(&fixture.render(140, 40));
    assert!(screen.contains("6041-1790605051159132520-1"), "{screen}");
    assert!(screen.contains("r1 · task #1"), "{screen}");
}

fn finished_fixture() -> Fixture {
    finished_fixture_with("Approved check failed with exit 1", None)
}

fn finished_fixture_with(
    detail: &str,
    check: Option<alfredo_tui::execution::ExecutionReceipt>,
) -> Fixture {
    let fixture = Fixture::new(vec![]);
    let store = fixture.store.clone();
    let mut snapshot = store.snapshot().unwrap();
    let mut act = |action: Action| {
        snapshot = store
            .transact(Request {
                correlation: format!("finish-{}", snapshot.revision + 1),
                expected_revision: snapshot.revision,
                action,
            })
            .unwrap()
            .0;
    };
    act(Action::Propose {
        title: "Create calc.py".into(),
        model: "fixture".into(),
        dependencies: vec![],
    });
    act(Action::Permit {
        task: 1,
        policy: WorkPolicy {
            files: vec!["calc.py".into()],
            check: vec!["/bin/true".into()],
        },
    });
    act(Action::Approve { task: 1 });
    let owner = store.claim_worker(1).unwrap();
    act(Action::Start {
        task: 1,
        baseline: "a".repeat(40),
        inputs: vec![],
    });
    let snapshot_now = store.snapshot().unwrap();
    let run = snapshot_now.tasks[0].run.as_ref().unwrap().id.clone();
    let evidence = serde_json::to_string(&Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None,
        generation: None,
        run: run.clone(),
        baseline: "a".repeat(40),
        status: TaskStatus::Failed,
        detail: detail.into(),
        patch: "diff --git a/calc.py b/calc.py\n+def add(a, b):\n+    return a + b".into(),
        check,
    })
    .unwrap();
    let directory = store.run_directory(&run).unwrap();
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("evidence.json"), &evidence).unwrap();
    act(Action::Finish {
        task: 1,
        run,
        status: TaskStatus::Failed,
        evidence_sha256: format!("{:x}", Sha256::digest(evidence.as_bytes())),
        detail: detail.into(),
    });
    drop(owner);
    let mut fixture = fixture;
    fixture.control.snapshot = Some(store.snapshot().unwrap());
    fixture.select(1);
    fixture
}

#[test]
fn finished_task_shows_outcome_then_diff_and_failure_reason_in_the_pane() {
    let fixture = finished_fixture();
    let screen = text(&fixture.render(140, 40));
    let outcome = screen
        .find("Approved check failed with exit 1")
        .expect(&screen);
    let diff = screen.find("+def add(a, b):").expect(&screen);
    assert!(outcome < diff, "{screen}");
    assert!(!screen.contains("finish-"), "{screen}");
}

fn failed_check(stdout: &str, stderr: &str) -> alfredo_tui::execution::ExecutionReceipt {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "request_id": "check:run", "request_digest": "d", "effect": "local-agent",
        "status": "failed", "started_at": "0", "ended_at": "1", "exit_code": 1,
        "stdout": stdout, "stderr": stderr, "stdout_bytes": stdout.len(), "stderr_bytes": stderr.len(),
        "stdout_sha256": "", "stderr_sha256": "", "effect_started": true, "reconciliation_required": false,
        "error_code": "", "error_message": "", "receipt_id": "r", "owner_pid": null, "owner_identity": "",
        "process_pid": null, "process_identity": "", "provider": "fixture"
    }))
    .unwrap()
}

#[test]
fn failed_task_pane_shows_the_check_output_tail_under_the_outcome_before_the_diff() {
    let stderr = "F\n======\nFAIL: test_add (test_calc.T.test_add)\nTraceback (most recent call last):\n  File \"/home/u/.state/runs/task-1-run-1/worktree/test_calc.py\", line 6, in test_add\n    self.assertEqual(add(1, 2), 3)\nAssertionError: 4 != 3\n\nFAILED (failures=1)\n";
    let detail = "Check failed (exit 1): stderr: F | ====== | FAIL: test_add | AssertionError: 4 != 3 | FAILED (failures=1)";
    let fixture = finished_fixture_with(detail, Some(failed_check("", stderr)));
    for (width, height) in [(80, 24), (100, 30)] {
        let buffer = fixture.render(width, height);
        assert_borders_closed(&buffer);
        let screen = text(&buffer);
        let outcome = screen
            .find("Result   Run failed · Check failed (exit 1)")
            .unwrap_or_else(|| panic!("{width}x{height}\n{screen}"));
        let label = screen.find("stderr · last lines").expect(&screen);
        let file = screen.find("File \"test_calc.py\", line 6").expect(&screen);
        assert!(outcome < label && label < file, "{screen}");
        assert!(!screen.contains("Check · failed"), "said once: {screen}");
        // The final error line is on screen at 100x30 (and one scroll away at 80x24).
        let error = screen.find("AssertionError: 4 != 3");
        assert!(
            height < 30 || error.is_some_and(|error| file < error),
            "{screen}"
        );
        assert!(!screen.contains("/worktree/"), "{screen}");
        assert!(!screen.contains("stderr: F | ======"), "{screen}");
        if let Some(diff) = screen.find("+def add(a, b):") {
            assert!(file < diff, "{screen}");
        }
    }
    // Only stdout: labelled stdout; an older detail with an empty colon reads cleanly.
    let fixture = finished_fixture_with(
        "Check failed (exit 1):",
        Some(failed_check("STDOUT_SENTINEL failure\n", "")),
    );
    let screen = text(&fixture.render(100, 30));
    assert!(
        screen.contains("Result   Run failed · Check failed (exit 1) "),
        "{screen}"
    );
    assert!(!screen.contains("(exit 1):"), "{screen}");
    let label = screen.find("stdout · last lines").expect(&screen);
    assert!(label < screen.find("STDOUT_SENTINEL").unwrap(), "{screen}");
}

#[test]
fn successful_outcome_reads_as_passed_not_as_a_pending_review_request() {
    let evidence = serde_json::to_string(&Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None,
        generation: None,
        run: "run-1".into(),
        baseline: "a".repeat(40),
        status: TaskStatus::ReviewReady,
        detail: "Approved check passed; changes await human review".into(),
        patch: String::new(),
        check: None,
    })
    .unwrap();
    let lines = alfredo_tui::dashboard::outcome_lines(&evidence).unwrap();
    let first = lines[0].to_string();
    assert!(first.contains("✓ Check passed"), "{first}");
    assert!(!first.contains("await human review"), "{first}");
    assert!(!lines.iter().any(|line| line.to_string().contains("run-1")));
}

#[test]
fn empty_chat_suggests_go_even_after_the_workspace_arrival_line() {
    let fixture = calc_plan();
    let mut app = App::new("fixture".into());
    let request = alfredo_tui::selection_command::Request {
        correlation: "arrival".into(),
        origin: alfredo_tui::selection_command::Origin::Conversation {
            workspace: "/repo/source".into(),
            mission: "source".into(),
            conversation: "default".into(),
            session: 0,
        },
        choice: alfredo_tui::selection_command::Choice {
            workspace: alfredo_tui::selection_command::WorkspaceChoice::Create {
                parent: "/repo".into(),
                name: "created".into(),
            },
            mission: alfredo_tui::selection_command::MissionChoice::StartNew {
                name: "next".into(),
            },
        },
        conversation: "default".into(),
    };
    app.sessions[0]
        .submit_selection_arrival(
            request,
            alfredo_tui::selection_command::Outcome {
                phase: alfredo_tui::selection_command::Phase::Selected,
                failure: None,
            },
        )
        .unwrap();
    let mut control = TaskControl::new(fixture.store.clone());
    control.snapshot = fixture.control.snapshot.clone();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
        .unwrap();
    let screen = text(terminal.backend().buffer());
    assert!(screen.contains("/go GOAL"), "{screen}");
}

#[test]
fn autopilot_follows_the_running_task_unless_the_user_recently_moved() {
    let mut fixture = calc_plan();
    fixture.select(1);
    fixture.control.autopilot = Some(AutopilotStatus {
        state: RunState::Running,
        goal: "calc".into(),
        done: 1,
        total: 6,
        failed: 1,
        repairs: 0,
        elapsed: Duration::from_secs(3),
        branch: None,
    });
    assert!(fixture.control.follow_running_task());
    assert_eq!(fixture.control.selected_task().unwrap().id, 2);
    // A manual move holds the user's selection.
    fixture.control.select_task(true);
    let chosen = fixture.control.selected_task().map(|task| task.id);
    assert!(!fixture.control.follow_running_task());
    assert_eq!(fixture.control.selected_task().map(|task| task.id), chosen);
    // ... until the hold expires.
    fixture
        .control
        .expire_manual_selection(Duration::from_secs(11));
    assert!(fixture.control.follow_running_task());
    assert_eq!(fixture.control.selected_task().unwrap().id, 2);
    // Without an active autopilot nothing moves.
    fixture.control.autopilot = None;
    fixture.select(1);
    assert!(!fixture.control.follow_running_task());
}

fn autopilot_status(state: RunState, branch: Option<&str>) -> AutopilotStatus {
    AutopilotStatus {
        state,
        goal: "create calc.py with add and sub and test_calc.py with unittest; check python3 -m unittest test_calc.py".into(),
        done: 2,
        total: 3,
        failed: 1,
        repairs: 0,
        elapsed: Duration::from_secs(83),
        branch: branch.map(Into::into),
    }
}

#[test]
fn header_is_status_mission_and_one_compact_autopilot_row() {
    let mut fixture = calc_plan();
    fixture.app.health = HealthView::observed(Health::Ready {
        model: "qwen2.5-coder:14b".into(),
    });
    fixture.control.autopilot = Some(autopilot_status(RunState::Running, None));
    for width in [80, 100, 140] {
        let buffer = fixture.render(width, 30);
        let lines = rows(&buffer);
        // Row 1: mission, repository name, attention only when non-zero, health.
        assert!(lines[0].contains("ALFREDO"), "{}", lines[0]);
        assert!(lines[0].contains("Dashboard · workspace"), "{}", lines[0]);
        assert!(lines[0].contains("1 review"), "{width}: {}", lines[0]);
        assert!(!lines[0].contains("dispatch"), "{width}: {}", lines[0]);
        assert!(
            lines[0].trim_end().ends_with("warm"),
            "{width}: {}",
            lines[0]
        );
        // Row 2: autopilot facts separated by spaces; no goal text.
        let autopilot = &lines[1];
        assert!(autopilot.contains("▶ running"), "{autopilot}");
        assert!(autopilot.contains("   2/3   "), "{autopilot}");
        assert!(autopilot.contains("1 failed"), "{autopilot}");
        assert!(autopilot.contains("01:23"), "{autopilot}");
        assert!(!autopilot.contains("create calc.py"), "{autopilot}");
        assert!(!autopilot.contains(" · "), "{autopilot}");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("Autopilot"))
                .count(),
            1
        );
    }
    fixture.control.autopilot = Some(autopilot_status(RunState::Done, Some("alfredo/go-7")));
    let lines = rows(&fixture.render(140, 30));
    assert!(lines[1].contains("✓ done"), "{}", lines[1]);
    assert!(
        lines[1].trim_end().ends_with("01:23   alfredo/go-7"),
        "{}",
        lines[1]
    );
}

#[test]
fn health_stays_right_aligned_on_the_mission_row_with_a_task_snapshot() {
    let mut fixture = calc_plan();
    for (state, label) in [
        (
            Health::Ready {
                model: "qwen2.5-coder:14b".into(),
            },
            "warm",
        ),
        (
            Health::Down {
                since: std::time::Instant::now(),
            },
            "ollama ✗ retrying",
        ),
    ] {
        fixture.app.health = HealthView::observed(state);
        for width in [80, 100, 140] {
            let lines = rows(&fixture.render(width, 30));
            // Mission and health share the single header row.
            assert!(
                lines[0].starts_with(" ALFREDO  Dashboard · workspace"),
                "{width}: {}",
                lines[0]
            );
            assert!(lines[0].contains(label), "{width}: {}", lines[0]);
            assert!(
                lines[0].trim_end().ends_with(label),
                "{width}: {}",
                lines[0]
            );
            assert!(!lines[1].contains("ollama"), "{width}: {}", lines[1]);
        }
    }
}

#[test]
fn layouts_fit_supported_sizes_and_degrade_at_40_by_12() {
    let mut fixture = calc_plan();
    fixture.select(2);
    fixture.control.autopilot = Some(autopilot_status(RunState::Running, None));
    for (width, height) in [(80, 24), (100, 30), (140, 40), (40, 12)] {
        for visible in [true, false] {
            fixture.control.visible = visible;
            let buffer = fixture.render(width, height);
            assert_borders_closed(&buffer);
            let lines = rows(&buffer);
            let footer = &lines[usize::from(height) - 2];
            assert!(
                unicode_width::UnicodeWidthStr::width(footer.trim_end()) <= usize::from(width),
                "{footer}"
            );
            assert!(footer.contains("F1"), "{width}x{height}: {footer}");
            // No hint is cut mid-word: the line ends with a complete hint.
            assert!(
                footer.trim_end().ends_with("quit") || footer.trim_end().ends_with("help"),
                "{width}x{height}: {footer}"
            );
        }
    }
    fixture.control.visible = true;
    let tiny = text(&fixture.render(40, 12));
    assert!(tiny.contains("#2"), "{tiny}");
}

#[test]
fn session_list_uses_short_status_words() {
    use alfredo_tui::model::Update;
    let mut app = App::new("qwen2.5-coder:14b".into());
    app.add_session();
    app.add_session();
    app.sessions[1].insert("hello");
    app.sessions[1].begin().unwrap();
    app.sessions[1].apply(1, Update::Admitted);
    app.sessions[2].insert("hello");
    app.sessions[2].begin().unwrap();
    app.sessions[2].apply(1, Update::Token("partial".into()));
    for (width, height) in [(80, 24), (100, 30), (140, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen = text(&buffer);
        if width < 88 {
            // Narrow: the summary row counts active chats; F6 shows the rows.
            assert!(screen.contains("2 chats active"), "{screen}");
            app.pane.toggle_focus(true);
            terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
            app.pane.toggle_focus(true);
        }
        let buffer = terminal.backend().buffer().clone();
        let screen = text(&buffer);
        let row = |label: &str| {
            screen
                .lines()
                .find(|line| line.contains(label))
                .unwrap_or_else(|| panic!("{label}: {screen}"))
                .to_owned()
        };
        assert!(row("chat 1").contains(" ready"), "{screen}");
        assert!(row("chat 2").contains(" thinking"), "{screen}");
        assert!(row("chat 3").contains(" streaming"), "{screen}");
        assert!(!screen.contains("Waiting for model ser"), "{screen}");
        assert_borders_closed(&buffer);
    }
}

#[test]
fn f1_help_is_grouped_with_go_first_and_fits_80_by_24() {
    let mut app = App::new("fixture".into());
    app.completion = Some(alfredo_tui::commands::Completion::all());
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let screen = text(&buffer);
    let autopilot = screen.find("Autopilot").expect(&screen);
    let go = screen.find("/go").expect(&screen);
    let tasks = screen.find("Tasks").expect(&screen);
    assert!(autopilot < go && go < tasks, "{screen}");
    assert_borders_closed(&buffer);
    // The rest is reachable by scrolling the selection.
    let completion = app.completion.as_mut().unwrap();
    for _ in 0..completion.choices.len() - 1 {
        completion.next(true);
    }
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let screen = text(terminal.backend().buffer());
    assert!(screen.contains("Advanced"), "{screen}");
    let groups: Vec<_> = app
        .completion
        .as_ref()
        .unwrap()
        .choices
        .iter()
        .filter_map(|choice| choice.group)
        .collect();
    let mut order = groups.clone();
    order.dedup();
    assert_eq!(
        order,
        [
            "Autopilot",
            "Tasks",
            "Review",
            "Chat",
            "Navigation",
            "Advanced"
        ]
    );
    assert_eq!(app.completion.as_ref().unwrap().choices[0].name, "/go");
}

#[test]
fn chat_failure_reason_stays_in_the_transcript_until_the_next_attempt() {
    use alfredo_tui::model::Update;
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("hello");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Failed("connection refused".into()));
    app.notice = "unrelated notice".into();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let transcript = region(&buffer, 20, 1, 80, 24);
    assert!(transcript.contains("connection refused"), "{transcript}");
    app.sessions[0].retry().unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    assert!(!text(terminal.backend().buffer()).contains("connection refused"));
}

#[test]
fn chat_timing_collapses_to_one_dim_line() {
    use alfredo_tui::model::Update;
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("hello");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Admitted);
    app.sessions[0].apply(1, Update::Token("answer".into()));
    app.sessions[0].apply(
        1,
        Update::Metrics(alfredo_tui::metrics::Metrics {
            eval_count: Some(49),
            eval_duration: Some(1_000_000_000),
            ..Default::default()
        }),
    );
    app.sessions[0].apply(1, Update::Done);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let screen = text(terminal.backend().buffer());
    assert!(screen.contains("49 tok/s"), "{screen}");
    assert!(!screen.contains("Client ·"), "{screen}");
    assert!(!screen.contains("Server timing"), "{screen}");
}

#[test]
fn autopilot_commands_keep_the_readers_scroll_position() {
    use alfredo_tui::model::Update;
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("Earlier question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Token("Earlier answer line\n".repeat(80)));
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].scroll_rows(-12);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let before = text(terminal.backend().buffer());
    let intent = alfredo_tui::command_intent::Intent::Task {
        request: Request {
            correlation: "autopilot-proposal".into(),
            expected_revision: 0,
            action: Action::Propose {
                title: "Autopilot task".into(),
                model: "fixture".into(),
                dependencies: vec![],
            },
        },
    };
    app.sessions[0]
        .submit_autopilot_command("/task Autopilot task".into(), intent)
        .unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let after = text(terminal.backend().buffer());
    assert!(!after.contains("/task Autopilot task"), "{after}");
    let body = |screen: &str| {
        screen
            .lines()
            .skip(1)
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(body(&before), body(&after));
}
