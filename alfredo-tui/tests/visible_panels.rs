//! Public rendering regressions; fixtures never execute a worker or contact a model.
use alfredo_tui::{
    commands::Completion,
    conversations::TaskView,
    model::App,
    review::View,
    task_control::TaskControl,
    tasks::{Action, Request, Snapshot, TaskStatus, TaskStore, WorkPolicy},
    ui, understanding,
    worker::Evidence,
};
use ratatui::{backend::TestBackend, buffer::Buffer, layout::Rect, Terminal};
use std::{
    collections::BTreeSet,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Panel {
    Inspector,
    Scope,
    Activity,
    Evidence,
    Planner,
}

const PANELS: [Panel; 5] = [
    Panel::Inspector,
    Panel::Scope,
    Panel::Activity,
    Panel::Evidence,
    Panel::Planner,
];

struct Fixture {
    root: PathBuf,
    store: TaskStore,
    control: TaskControl,
    app: App,
    saved: Snapshot,
    scope: understanding::Snapshot,
    sessions: serde_json::Value,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-visible-panels-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = TaskStore::new(&root.join("state"), &workspace, "Panel fixture").unwrap();
        for (revision, title) in [(0, "Unselected task"), (1, "Selected task")] {
            store
                .transact(Request {
                    correlation: format!("proposal-{revision}"),
                    expected_revision: revision,
                    action: Action::Propose {
                        title: title.into(),
                        model: "exact-worker-model".into(),
                        dependencies: vec![],
                    },
                })
                .unwrap();
        }
        for index in 0..12 {
            store
                .transact(Request {
                    correlation: format!("ACT_{index:02}"),
                    expected_revision: index + 2,
                    action: Action::Permit {
                        task: 2,
                        policy: WorkPolicy {
                            files: vec![format!("file-{index}.rs")],
                            check: vec!["true".into()],
                        },
                    },
                })
                .unwrap();
        }
        let saved = store.snapshot().unwrap();
        let scope = store
            .understanding()
            .transact(understanding::Request {
                correlation: "scope-draft".into(),
                expected_revision: 0,
                action: understanding::Action::Draft {
                    brief: understanding::Brief {
                        destination: markers("SCOPE", 24).join("\n"),
                        scope: "Inspect recorded work".into(),
                        constraints: "Keep current selection".into(),
                        uncertainty: "No execution requested".into(),
                    },
                },
            })
            .unwrap();
        let mut control = TaskControl::new(store.clone());
        control.snapshot = Some(saved.clone());
        control.canonical_scope = Some(scope.clone());
        control
            .restore_view(TaskView {
                visible: true,
                selected: Some(2),
                query: String::new(),
            })
            .unwrap();
        control.planner.notice = "DRAFT_HEADER".into();
        control.planner.partial = markers("PLAN", 24).join("\n");
        let mut app = App::new("conversation-model".into());
        app.sessions[0].insert("keep draft 界");
        let sessions = serde_json::to_value(&app.sessions).unwrap();
        Self {
            root,
            store,
            control,
            app,
            saved,
            scope,
            sessions,
        }
    }

    fn show(&mut self, winner: Panel, stacked: bool) {
        let shown = |panel| winner == panel || (stacked && panel < winner);
        self.control.scope_view = shown(Panel::Scope).then(|| self.scope.clone());
        self.control.activity = shown(Panel::Activity).then(|| "#2".into());
        self.control.evidence = shown(Panel::Evidence).then(evidence);
        self.control.planner.visible = shown(Panel::Planner);
    }

    fn draw(&self, terminal: &mut Terminal<TestBackend>) -> Buffer {
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &self.app, &self.control))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn maximum(&mut self) -> usize {
        self.control.scroll = usize::MAX;
        self.control.scroll_rows(0);
        self.control.scroll
    }

    fn page_step(&mut self) -> usize {
        self.control.scroll = 0;
        self.control.page_details(true);
        self.control.scroll
    }

    fn assert_unchanged(&self) {
        let saved = serde_json::to_value(&self.saved).unwrap();
        assert_eq!(
            serde_json::to_value(self.control.snapshot.as_ref().unwrap()).unwrap(),
            saved
        );
        assert_eq!(
            serde_json::to_value(self.store.snapshot().unwrap()).unwrap(),
            saved
        );
        assert_eq!(self.store.understanding().snapshot().unwrap(), self.scope);
        assert_eq!(self.control.canonical_scope.as_ref(), Some(&self.scope));
        assert_eq!(self.control.selected_task().map(|task| task.id), Some(2));
        assert_eq!(self.app.selected, 0);
        assert_eq!(
            serde_json::to_value(&self.app.sessions).unwrap(),
            self.sessions
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn markers(prefix: &str, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| format!("{prefix}_{index:02}"))
        .collect()
}

fn evidence() -> View {
    let raw = serde_json::to_string(&Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None,
        generation: None,
        run: "task-2-run-1".into(),
        baseline: "a".repeat(40),
        status: TaskStatus::Failed,
        detail: "Read-only evidence display fixture".into(),
        patch: markers("DIFF", 24).join("\n"),
        check: None,
    })
    .unwrap();
    View::from_verified(2, &raw).unwrap()
}

/// Resize the backend too: `Terminal::draw` autoresizes back to the backend size.
fn resize(terminal: &mut Terminal<TestBackend>, width: u16, height: u16) {
    terminal.backend_mut().resize(width, height);
    terminal.resize(Rect::new(0, 0, width, height)).unwrap();
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn stacked_task_panels_match_the_winning_panel_buffer_and_scroll_bounds() {
    let mut fixture = Fixture::new();
    let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
    for (width, height) in [(140, 40), (32, 10)] {
        resize(&mut terminal, width, height);
        for panel in PANELS {
            for scroll in [0, 7, usize::MAX] {
                fixture.show(panel, false);
                fixture.control.scroll = scroll;
                let isolated = fixture.draw(&mut terminal);
                let maximum = fixture.maximum();
                let step = fixture.page_step();
                fixture.show(panel, true);
                fixture.control.scroll = scroll;
                let stacked = fixture.draw(&mut terminal);
                assert_eq!(
                    stacked, isolated,
                    "{panel:?} at {width}x{height}, scroll={scroll}"
                );
                assert_eq!(fixture.maximum(), maximum, "{panel:?} scroll range");
                assert_eq!(fixture.page_step(), step, "{panel:?} page height");
                assert!(text(&stacked).contains("keep draft 界"));
            }
        }
    }
    fixture.assert_unchanged();
}

#[test]
fn task_panel_pages_reach_every_marker_in_both_directions_after_resize() {
    let mut fixture = Fixture::new();
    let mut terminal = Terminal::new(TestBackend::new(32, 10)).unwrap();
    for (panel, expected) in [
        (Panel::Planner, markers("PLAN", 24)),
        (Panel::Scope, markers("SCOPE", 24)),
        (Panel::Activity, markers("ACT", 12)),
        (
            Panel::Inspector,
            vec!["ACT_09".into(), "ACT_10".into(), "ACT_11".into()],
        ),
    ] {
        fixture.show(panel, true);
        fixture.control.scroll = usize::MAX;
        for (width, height) in [(32, 10), (140, 40), (32, 10)] {
            resize(&mut terminal, width, height);
            // The resize must replace the previous viewport's bounds before paging.
            fixture.draw(&mut terminal);
            let maximum = fixture.maximum();
            for forward in [false, true] {
                let mut seen = BTreeSet::new();
                let mut reached_boundary = false;
                for _ in 0..512 {
                    let screen = text(&fixture.draw(&mut terminal));
                    assert!(screen.contains("keep draft 界"));
                    for marker in &expected {
                        if screen.contains(marker) {
                            seen.insert(marker.clone());
                        }
                    }
                    let previous = fixture.control.scroll;
                    fixture.control.page_details(forward);
                    if previous == fixture.control.scroll {
                        reached_boundary = true;
                        break;
                    }
                }
                assert!(reached_boundary, "{panel:?} did not reach its bound");
                assert_eq!(
                    seen,
                    expected.iter().cloned().collect(),
                    "{panel:?} {width}x{height}, forward={forward} skipped content"
                );
                assert_eq!(fixture.control.scroll, if forward { maximum } else { 0 });
            }
        }
    }
    fixture.assert_unchanged();
}

#[test]
fn model_and_completion_overlays_preserve_task_paging_until_the_panel_returns() {
    let mut fixture = Fixture::new();
    let mut terminal = Terminal::new(TestBackend::new(32, 10)).unwrap();
    for (models, completion) in [(true, false), (false, true), (true, true)] {
        fixture.show(Panel::Planner, true);
        resize(&mut terminal, 32, 10);
        fixture.control.scroll = 0;
        fixture.draw(&mut terminal);
        let maximum = fixture.maximum();
        let step = fixture.page_step();

        fixture.app.models_visible = models;
        fixture.app.models = vec!["catalog-model".into()];
        fixture.app.models_notice = "MODEL_CATALOG".into();
        fixture.app.completion = completion.then(|| Completion::open("/ta").unwrap());
        resize(&mut terminal, 140, 40);
        let covered = fixture.draw(&mut terminal);
        assert!(text(&covered).contains(if completion { "Complete" } else { "Models" }));
        assert!(!text(&covered).contains("DRAFT_HEADER"));
        assert_eq!(
            fixture.maximum(),
            maximum,
            "covered panel changed scroll range"
        );
        assert_eq!(
            fixture.page_step(),
            step,
            "covered panel changed page height"
        );

        fixture.app.models_visible = false;
        fixture.app.completion = None;
        fixture.control.scroll = 0;
        let returned = fixture.draw(&mut terminal);
        assert!(text(&returned).contains("DRAFT_HEADER"));
        assert!(
            fixture.maximum() < maximum,
            "exposed panel retained narrow bounds"
        );
        fixture.show(Panel::Planner, false);
        fixture.control.scroll = 0;
        assert_eq!(fixture.draw(&mut terminal), returned);
    }
    fixture.assert_unchanged();
}
