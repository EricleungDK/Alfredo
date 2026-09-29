//! Agent view in the right pane: opening from the side pane, prompt targeting,
//! per-target drafts, rendering, reading position and the console commands.
use alfredo_tui::{
    agent_view::{self, Target},
    mission_work::NodeId,
    model::App,
    side_pane::{self, OpenTarget},
    task_control::TaskControl,
    tasks::{Action, Request, TaskStore, WorkPolicy},
    worker::Progress,
};
use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::watch;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    control: TaskControl,
    app: App,
    progress: watch::Sender<Progress>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const PROMPT: &str = "Implement this task: Make answer return 42\nAllowed exact files: [\"calc.py\"]\n\nFILE calc.py\ndef answer():\n    return 0\n\nREAD-ONLY REFERENCE FILES (committed baseline; reference only)\nREAD-ONLY FILE README.md\nCalculator\n";

/// One running task with a live worker streaming its answer.
fn fixture(output: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "alfredo-agent-pane-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "default").unwrap();
    for action in [
        Action::Propose {
            title: "Make answer return 42".into(),
            model: "qwen2.5-coder:14b".into(),
            dependencies: vec![],
        },
        Action::Permit {
            task: 1,
            policy: WorkPolicy {
                files: vec!["calc.py".into()],
                check: vec!["python3".into(), "-m".into(), "unittest".into()],
            },
        },
        Action::Approve { task: 1 },
        Action::Start {
            task: 1,
            baseline: "a".repeat(40),
            inputs: vec![],
        },
    ] {
        let revision = store.snapshot().unwrap().revision;
        store
            .transact(Request {
                correlation: format!("fixture-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap();
    }
    let mut control = TaskControl::new(store.clone());
    control.snapshot = Some(store.snapshot().unwrap());
    let (progress, receiver) = watch::channel(Progress {
        stage: "Receiving model plan",
        model_output: output.into(),
        request: Some(Arc::from(PROMPT)),
        ..Progress::default()
    });
    control.attach_progress(1, receiver, Arc::new(AtomicBool::new(false)));
    Fixture {
        root,
        control,
        app: App::new("qwen2.5-coder:14b".into()),
        progress,
    }
}

fn render(app: &App, control: &TaskControl, width: u16, height: u16) -> Vec<String> {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw_with_tasks(frame, app, control))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

/// Text of the right pane's inner area (after the side pane and one padding column).
fn right(rows: &[String], width: u16) -> Vec<String> {
    let start = alfredo_tui::ui::pane_width(width) as usize + 2;
    rows.iter()
        .map(|row| {
            row.chars()
                .skip(start)
                .collect::<String>()
                .trim_end_matches(['│', ' '])
                .to_string()
        })
        .collect()
}

fn code(lines: usize) -> String {
    let mut text = String::from("=== FILE: calc.py ===\n");
    for index in 0..lines {
        text.push_str(&format!("LINE_{index:03} = {index}\n"));
    }
    text
}

#[test]
fn enter_on_a_task_row_opens_its_agent_and_the_prompt_targets_it() {
    let mut fixture = fixture("=== FILE: calc.py ===\ndef answer():\n    return 42\n");
    fixture.app.sessions[0].insert("unsent chat line");
    fixture.control.set_visible(false);
    side_pane::open_work_target(
        &mut fixture.app,
        &mut fixture.control,
        &OpenTarget::Node(NodeId::Task(1)),
    );
    assert_eq!(
        fixture.control.agent_shown().map(|view| view.target),
        Some(Target::Task(1))
    );
    assert_eq!(
        fixture.app.sessions[0].draft, "",
        "the agent has its own draft"
    );
    for (width, height) in [(100, 30), (200, 50)] {
        let rows = render(&fixture.app, &fixture.control, width, height);
        let screen = rows.join("\n");
        assert!(
            screen.contains("┌ Agent · worker #1 · running "),
            "{screen}"
        );
        assert!(
            screen.contains(" To worker #1 · Enter send · Esc back "),
            "{screen}"
        );
        let pane = right(&rows, width);
        let at = pane
            .iter()
            .position(|row| row.starts_with("You → worker #1"))
            .unwrap_or_else(|| panic!("{screen}"));
        assert_eq!(
            pane[at..at + 12],
            [
                "You → worker #1",
                "Make answer return 42",
                "files calc.py · check python3 -m unittest",
                "",
                "References",
                "README.md",
                "",
                "Worker",
                "▸ calc.py",
                "def answer():",
                "    return 42",
                ""
            ],
            "{screen}"
        );
        // Instructions live in the title and footer, never in the content.
        let body = &pane[..pane.len() - 5];
        assert!(!body.iter().any(|row| row.contains("Esc")), "{screen}");
    }
}

#[test]
fn esc_restores_the_previous_pane_and_both_drafts_survive() {
    let mut fixture = fixture("");
    fixture.app.sessions[0].insert("unsent chat line");
    fixture.control.set_visible(false);
    agent_view::open(&mut fixture.app, &mut fixture.control, Target::Task(1));
    fixture.app.sessions[0].insert("half a note");
    agent_view::close(&mut fixture.app, &mut fixture.control, true);
    assert!(fixture.control.agent.is_none());
    assert!(!fixture.control.visible, "back to the chat it came from");
    assert_eq!(fixture.app.sessions[0].draft, "unsent chat line");
    agent_view::open(&mut fixture.app, &mut fixture.control, Target::Task(1));
    assert_eq!(fixture.app.sessions[0].draft, "half a note");
    // Leaving through another view keeps that view and still restores the chat draft.
    fixture.control.activity = Some(String::new());
    assert!(fixture.control.agent_shown().is_none());
    agent_view::close(&mut fixture.app, &mut fixture.control, false);
    assert!(fixture.control.visible);
    assert_eq!(fixture.app.sessions[0].draft, "unsent chat line");
}

#[test]
fn live_view_follows_the_tail_and_keeps_the_reading_position_when_scrolled_up() {
    let mut fixture = fixture(&code(80));
    agent_view::open(&mut fixture.app, &mut fixture.control, Target::Task(1));
    let pane = right(&render(&fixture.app, &fixture.control, 100, 30), 100);
    assert!(pane.iter().any(|row| row == "LINE_079 = 79"), "{pane:#?}");
    fixture.control.agent.as_ref().unwrap().scroll_rows(-10);
    let pane = right(&render(&fixture.app, &fixture.control, 100, 30), 100);
    assert!(!pane.iter().any(|row| row == "LINE_079 = 79"));
    let top = pane
        .iter()
        .find(|row| row.starts_with("LINE_"))
        .unwrap()
        .clone();
    // New streamed output does not move a reader who scrolled away.
    fixture
        .progress
        .send_modify(|progress| progress.model_output = code(90));
    let pane = right(&render(&fixture.app, &fixture.control, 100, 30), 100);
    assert_eq!(
        pane.iter().find(|row| row.starts_with("LINE_")).unwrap(),
        &top
    );
    // Back at the bottom it follows the stream again.
    fixture.control.agent.as_ref().unwrap().scroll_rows(1000);
    render(&fixture.app, &fixture.control, 100, 30);
    fixture
        .progress
        .send_modify(|progress| progress.model_output = code(95));
    let pane = right(&render(&fixture.app, &fixture.control, 100, 30), 100);
    assert!(pane.iter().any(|row| row == "LINE_094 = 94"), "{pane:#?}");
}

#[test]
fn watch_and_tell_work_from_anywhere() {
    let mut fixture = fixture("");
    let (app, control) = (&mut fixture.app, &mut fixture.control);
    control.set_visible(false);
    assert_eq!(
        agent_view::console(app, control, "/watch 1"),
        Some(Ok("Watching worker #1".into()))
    );
    assert_eq!(control.agent_shown().unwrap().target, Target::Task(1));
    assert_eq!(
        agent_view::console(app, control, "/watch 7"),
        Some(Err("Task #7 not found".into()))
    );
    assert_eq!(
        agent_view::console(app, control, "/watch architect"),
        Some(Err(
            "No plan is being drafted; /plan REQUEST starts one".into()
        ))
    );
    assert!(agent_view::console(app, control, "/tell 1")
        .unwrap()
        .is_err());
    assert_eq!(
        agent_view::console(app, control, "/tell 1 use the integer 42"),
        Some(Ok("Steering #1 · cancelling the generation".into()))
    );
    assert_eq!(control.owner.instructions()[0].note, "use the integer 42");
    assert_eq!(agent_view::console(app, control, "/tasks"), None);
}
