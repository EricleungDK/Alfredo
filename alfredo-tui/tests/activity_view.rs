//! F4 activity view: pinned visual output, cache reuse and windowed rendering.
use alfredo_tui::{
    model::App,
    task_control::TaskControl,
    tasks::{Action, Receipt, Request, Snapshot, TaskStore},
    ui,
};
use ratatui::{backend::TestBackend, Terminal};
use sha2::{Digest, Sha256};

fn control(count: u64) -> (TaskControl, std::path::PathBuf) {
    // Tests run in parallel threads of one process: each fixture needs its own
    // directory, or one test's cleanup deletes another's workspace.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "alfredo-activity-view-{}-{count}-{id}",
        std::process::id()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "activity-view").unwrap();
    let mut tasks = TaskControl::new(store);
    tasks.visible = true;
    let receipts: Vec<Receipt> = (1..=count)
        .map(|revision| Receipt {
            revision,
            task: 1 + revision % 5,
            request: Request {
                correlation: format!(
                    "correlation-{revision}-{}",
                    "x".repeat((revision % 7) as usize * 9)
                ),
                expected_revision: revision - 1,
                action: if revision % 3 == 0 {
                    Action::Approve {
                        task: 1 + revision % 5,
                    }
                } else {
                    Action::Propose {
                        title: format!(
                            "Receipt {revision} {} 界界界 tail",
                            "wrapped words in a long title ".repeat((revision % 6) as usize + 1)
                        ),
                        model: "worker".into(),
                        dependencies: vec![],
                    }
                },
            },
        })
        .collect();
    tasks.snapshot = Some(Snapshot {
        schema_version: alfredo_tui::tasks::SCHEMA_VERSION,
        workspace,
        mission: "activity-view".into(),
        revision: count,
        tasks: vec![],
        receipts,
    });
    (tasks, root)
}

fn render(tasks: &TaskControl, width: u16, height: u16) -> String {
    let app = App::new("fixture".into());
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &app, tasks))
        .unwrap();
    format!("{:?}", terminal.backend().buffer())
}

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Hashes recorded from the unwindowed renderer before the cache existed (34x11
/// recorded from the original renderer in a scratch checkout). Regenerate them
/// only after confirming that a visual change to the F4 view is intended.
#[test]
fn rendered_output_matches_the_unwindowed_renderer() {
    const GOLDEN: &[(u16, u16, usize, &str)] = &[
        (
            140,
            40,
            0,
            "20a017914f62f0df679e7da44ea83449a2c9d83daabe82535b966b73a02f89bf",
        ),
        (
            140,
            40,
            1,
            "58dc701b1dde5113f0acdd150a227cabaa0e06bb0fe53ad0f95fabb8f4ce8634",
        ),
        (
            140,
            40,
            7,
            "c148494c2201782e6f8b1f108f2d086d12afd65b1004bbe919256c60165224ae",
        ),
        (
            140,
            40,
            100,
            "b1b014948ca5b2e55ff0137b1661afd79b2bf9d57fa6cf9f1e4f20421049f0e5",
        ),
        (
            140,
            40,
            100000,
            "180ed6b22ad8151437bc4119cf45fa510b75dfe829c854ae4e144f6d86180709",
        ),
        (
            80,
            24,
            0,
            "47a5e4087809716980267303a2baae9c5a4ac9d7511bfdc78595d4b076e0ba5e",
        ),
        (
            80,
            24,
            1,
            "a784f8a20b40576dd9042a8add950172ecec2223fe78c08080b161bba0d82a83",
        ),
        (
            80,
            24,
            7,
            "d61a932ab2a8ab6d2434351e78a8efc43cf6fb1aafdf0f025c552361bb15cf9e",
        ),
        (
            80,
            24,
            100,
            "38e51ca70f12a5eb4a4b6ea8dd58980552957ce7a118f9c23c130b72c8112d4f",
        ),
        (
            80,
            24,
            100000,
            "5ad55bec0121bd6087f437c859b00e8e55a766c5dc0ca72d52cba93965a07b3b",
        ),
        (
            32,
            10,
            0,
            "69f148d08d89b72aa0d4b1228514085c9090fac26fbabbce2a50c21cb4919584",
        ),
        (
            32,
            10,
            1,
            "2680b651371213da33ca9852b48aaf5442b2e78f1d118ecf100686c62158ad45",
        ),
        (
            32,
            10,
            7,
            "f475804987d355ac6031e70af5fb3139a350ca0e7fc1e0c85af4ce7499d3a8ff",
        ),
        (
            32,
            10,
            100,
            "f3a7d6f0bba0d790262f0085bc1b713f6b17d71339f338aedde8985e31a20b1f",
        ),
        (
            32,
            10,
            100000,
            "d5a4eec05d9e9f677ad26bac6092e3cc1e4211ac87d3eb383d234477ce378a2a",
        ),
        (
            34,
            11,
            0,
            "8ab4f21babfcdcb18d56408afa2fa00f11c71c304aaf1d8b13e8601daceb54fc",
        ),
        (
            34,
            11,
            1,
            "ada3f3968ee99c8ca2606468e487b551b156d1d68a04fc92bd5d9ee48d6b7796",
        ),
        (
            34,
            11,
            7,
            "096e054bd7293c16a49995f967abebddafb8b65846f6785579acd72fb77488d4",
        ),
        (
            34,
            11,
            100,
            "1f28407d5759c208d815feddeee2a8cdeb56f1c480b4a3798a8fac09a827e106",
        ),
        (
            34,
            11,
            100000,
            "93025d57c1a361af8b6bc2552e64b8980eaf7e9a09b08df0686df38ebcc0556b",
        ),
    ];
    let (mut tasks, root) = control(60);
    tasks.activity = Some(String::new());
    let mut actual = vec![];
    for (width, height) in [(140, 40), (80, 24), (32, 10), (34, 11)] {
        for scroll in [0, 1, 7, 100, 100_000] {
            tasks.scroll = scroll;
            actual.push((
                width,
                height,
                scroll,
                digest(&render(&tasks, width, height)),
            ));
        }
    }
    assert_eq!(actual.len(), GOLDEN.len());
    for ((w, h, s, d), (gw, gh, gs, gd)) in actual.iter().zip(GOLDEN) {
        assert_eq!((w, h, s, d.as_str()), (gw, gh, gs, *gd));
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unchanged_revision_query_and_width_reuse_the_cached_entries() {
    let (mut tasks, root) = control(40);
    tasks.activity = Some("#2".into());
    assert_eq!(tasks.activity_builds(), 0);
    let first = render(&tasks, 100, 30);
    assert_eq!(tasks.activity_builds(), 1);
    for scroll in [0, 3, 9] {
        tasks.scroll = scroll;
        render(&tasks, 100, 30);
        render(&tasks, 60, 20); // another width re-wraps rows, not entries
    }
    assert_eq!(tasks.activity_builds(), 1);
    tasks.scroll = 0;
    assert_eq!(render(&tasks, 100, 30), first);
    // A new query or a new revision rebuilds exactly once.
    tasks.activity = Some("#3".into());
    render(&tasks, 100, 30);
    render(&tasks, 100, 30);
    assert_eq!(tasks.activity_builds(), 2);
    let snapshot = tasks.snapshot.as_mut().unwrap();
    let mut receipt = snapshot.receipts.last().unwrap().clone();
    receipt.revision += 1;
    receipt.task = 3;
    receipt.request.correlation = "newest-receipt".into();
    snapshot.receipts.push(receipt);
    snapshot.revision += 1;
    assert!(render(&tasks, 100, 30).contains("newest-receipt"));
    assert_eq!(tasks.activity_builds(), 3);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_frame_materializes_only_the_visible_window() {
    let (tasks, root) = control(4096);
    let snapshot = tasks.snapshot.as_ref().unwrap();
    let mut view = alfredo_tui::activity::View::default();
    for (width, height, offset) in [(140, 38, 0), (30, 8, 20_000), (30, 8, usize::MAX)] {
        let window = view.window(snapshot, "", width, height, offset);
        assert!(window.maximum > 1000);
        assert!(
            window.lines.len() <= usize::from(height) + 6,
            "{}",
            window.lines.len()
        );
    }
    assert_eq!(view.builds(), 1);
    let _ = std::fs::remove_dir_all(root);
}

/// Poll until the in-flight background read has been consumed; true if any poll
/// reported a visible change.
fn settle(tasks: &mut TaskControl) -> bool {
    let mut changed = false;
    for _ in 0..500 {
        changed |= tasks.poll();
        if !tasks.refresh_in_flight() {
            return changed;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("background refresh never completed");
}

#[test]
fn an_unchanged_refresh_tick_does_not_request_a_redraw() {
    let (mut tasks, root) = control(0);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    tasks.snapshot = None;
    tasks.refresh_background(&runtime);
    assert!(settle(&mut tasks), "first load changes the screen");
    // Same revision, observations and gate: the result is consumed, nothing moved.
    tasks.refresh_background(&runtime);
    assert!(
        !settle(&mut tasks),
        "identical refresh must not mark the UI dirty"
    );
    // A new durable receipt (another writer) is a visible change.
    TaskStore::new(
        &root.join("state"),
        &root.join("workspace"),
        "activity-view",
    )
    .unwrap()
    .transact(Request {
        correlation: "outside-writer".into(),
        expected_revision: 0,
        action: Action::Propose {
            title: "Added elsewhere".into(),
            model: "worker".into(),
            dependencies: vec![],
        },
    })
    .unwrap();
    tasks.refresh_background(&runtime);
    assert!(settle(&mut tasks), "a revision change must redraw");
    // So is a notice change carried by a failed read.
    std::fs::remove_dir_all(root.join("state")).ok();
    std::fs::write(root.join("state"), b"not a directory").unwrap();
    tasks.refresh_background(&runtime);
    assert!(settle(&mut tasks), "a refresh failure notice must redraw");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn live_progress_requests_timing_redraws_even_when_the_pane_is_hidden() {
    let (mut tasks, root) = control(0);
    tasks.visible = false;
    assert!(!tasks.timing_redraw_due());
    let (_sender, receiver) = tokio::sync::watch::channel(alfredo_tui::worker::Progress::default());
    tasks.attach_progress(
        1,
        receiver,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    assert!(tasks.timing_redraw_due(), "elapsed clocks must keep moving");
    tasks.detach_progress(1);
    assert!(!tasks.timing_redraw_due());
    let _ = std::fs::remove_dir_all(root);
}

fn finish_receipt(status: alfredo_tui::tasks::TaskStatus, detail: &str) -> Receipt {
    Receipt {
        revision: 1,
        task: 1,
        request: Request {
            correlation: "finish:task-1-run-1".into(),
            expected_revision: 0,
            action: Action::Finish {
                task: 1,
                run: "task-1-run-1".into(),
                status,
                evidence_sha256: "b".repeat(64),
                detail: detail.into(),
            },
        },
    }
}

#[test]
fn a_failed_finish_row_carries_the_receipt_cause_cut_with_an_ellipsis() {
    use alfredo_tui::tasks::TaskStatus::Failed;
    let detail = "Check failed (exit 3): stderr: AssertionError: v should be 1 but is 0";
    assert_eq!(
        ui::short_phase(&finish_receipt(Failed, detail)),
        "Task #1 failed · Check failed (exit 3): stderr: AssertionError: v should be 1 b…"
    );
}

const TRACEBACK_DETAIL: &str = "Check failed (exit 1): stderr: Traceback (most recent call last): | File \"<string>\", line 1, in <module> | from calc import answer; assert answer() == 42 | ^^^^^^^^^^^^^^ | AssertionError";

#[test]
fn a_failed_finish_row_of_a_python_traceback_names_the_key_error_line() {
    use alfredo_tui::tasks::TaskStatus::Failed;
    let row = ui::short_phase(&finish_receipt(Failed, TRACEBACK_DETAIL));
    assert_eq!(
        row,
        "Task #1 failed · Check failed (exit 1): AssertionError"
    );
}

#[test]
fn a_failed_finish_row_keeps_the_key_line_when_a_long_check_head_is_cut() {
    use alfredo_tui::tasks::TaskStatus::Failed;
    let detail = format!(
        "Check failed (exit 1): {}: stderr: noise | ImportError: cannot import name 'answer'",
        "python -c from calc import answer ".repeat(4)
    );
    let row = ui::short_phase(&finish_receipt(Failed, &detail));
    assert!(
        row.ends_with("ImportError: cannot import name 'answer'"),
        "{row}"
    );
    assert!(
        row.contains('…'),
        "the head is cut, not the key line: {row}"
    );
    assert!(row.chars().count() <= 80, "{row}");
    assert!(!row.contains("stderr:"), "{row}");
}

#[test]
fn the_failed_finish_cause_is_cut_one_cell_past_the_limit_not_at_it() {
    use alfredo_tui::tasks::TaskStatus::Failed;
    let prefix = "Task #1 failed · ";
    let room = 80 - prefix.chars().count();
    let fits = "x".repeat(room);
    assert_eq!(
        ui::short_phase(&finish_receipt(Failed, &fits)),
        format!("{prefix}{fits}")
    );
    let over = "x".repeat(room + 1);
    assert_eq!(
        ui::short_phase(&finish_receipt(Failed, &over)),
        format!("{prefix}{}…", "x".repeat(room - 1))
    );
}

#[test]
fn finishes_other_than_failed_keep_their_rows_whatever_the_detail() {
    use alfredo_tui::tasks::TaskStatus::{Cancelled, NeedsHumanReview, ReviewReady};
    for (status, row) in [
        (ReviewReady, "Task #1 check passed · awaiting review"),
        (Cancelled, "Task #1 run cancelled"),
        (NeedsHumanReview, "Task #1 held for human review"),
    ] {
        assert_eq!(
            ui::short_phase(&finish_receipt(status, "Check failed (exit 3)")),
            row
        );
    }
}

#[test]
fn a_failed_finish_with_no_cause_keeps_the_bare_row() {
    use alfredo_tui::tasks::TaskStatus::Failed;
    for detail in ["", "   ", "\n\t"] {
        assert_eq!(
            ui::short_phase(&finish_receipt(Failed, detail)),
            "Task #1 failed",
            "{detail:?}"
        );
    }
}

#[test]
fn a_failed_finish_cause_is_one_line_with_hidden_characters_shown() {
    use alfredo_tui::tasks::TaskStatus::Failed;
    let row = ui::short_phase(&finish_receipt(
        Failed,
        "Check failed\nstderr: a\u{202e}b\u{200b}c\r\nd\u{2028}e",
    ));
    assert_eq!(
        row,
        "Task #1 failed · Check failed stderr: a<U+202E>b<U+200B>c d e"
    );
}

#[test]
fn a_failed_check_receipt_shows_its_whole_key_line_at_100_columns() {
    let key = "AssertionError: answer was 41 not 42";
    let detail = format!("Check failed (exit 1): stderr: {key}");
    let (mut tasks, root) = control(0);
    let snapshot = tasks.snapshot.as_mut().unwrap();
    snapshot.receipts.push(Receipt {
        revision: 1,
        task: 1,
        request: Request {
            correlation: "finish:task-1-run-1".into(),
            expected_revision: 0,
            action: Action::Finish {
                task: 1,
                run: "task-1-run-1".into(),
                status: alfredo_tui::tasks::TaskStatus::Failed,
                evidence_sha256: "b".repeat(64),
                detail: format!("No change from previous attempt · {detail}"),
            },
        },
    });
    snapshot.revision = 1;
    // The stored entry holds the whole line; nothing cuts it where it is built.
    let entries = alfredo_tui::activity::entries(snapshot, "");
    assert!(entries[0].detail.contains(&detail), "{}", entries[0].detail);
    tasks.activity = Some(String::new());
    let app = App::new("fixture".into());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| ui::draw_with_tasks(frame, &app, &tasks))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let screen = (0..buffer.area.height)
        .map(|y| {
            (ui::pane_width(100) + 2..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
        .replace('│', " ");
    let joined = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(joined.contains(&detail), "{joined}");
    let _ = std::fs::remove_dir_all(root);
}
