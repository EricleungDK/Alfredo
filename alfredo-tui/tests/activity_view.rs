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
    let root = std::env::temp_dir().join(format!(
        "alfredo-activity-view-{}-{count}",
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
            "aababf97de736480ff55cdf7879187c443f083789001d5095cc1f34d43705120",
        ),
        (
            140,
            40,
            1,
            "a77a6996fbafd63a50b82193f650ece61843842e55ae5cbe3cf3818ca5b7bbc7",
        ),
        (
            140,
            40,
            7,
            "4c54f485752a0796fac3c199097adf00b9b1ead2d9efea19a9cfc2845132b701",
        ),
        (
            140,
            40,
            100,
            "92ac699a352a49b86a236989f6084d31a3a5d1866ab1a3cbf739688bdd43245c",
        ),
        (
            140,
            40,
            100000,
            "36f10dddcca94853c5dd31c8b69bcf7d43ce9220ae943119d09b0dc45a8e43fc",
        ),
        (
            80,
            24,
            0,
            "f9394baa5cfe90f3162253295f8c8226e2023d6ffa9f00631971ab81388c5cd3",
        ),
        (
            80,
            24,
            1,
            "dbd0ecf48c13e1b4ac625c9f90bc933ace68fa75fb3dac171b5ce7479e19b2b9",
        ),
        (
            80,
            24,
            7,
            "9a82b9c714dd50295f83e7488c51954747f3f9eb584174c1dde43caa1967c2d8",
        ),
        (
            80,
            24,
            100,
            "8d45b9eefe58f819d0eddce2ffb3feb85a4fea14d3e94e831e38b78f1ffbbadd",
        ),
        (
            80,
            24,
            100000,
            "27f678be6a06b77485f1eb1579f79c0b04507bc5dec63948b928cf15b8da9916",
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
