use alfredo_tui::{
    model::App,
    review::View,
    task_control::TaskControl,
    tasks::{TaskStatus, TaskStore},
    ui,
    worker::Evidence,
};
use ratatui::{backend::TestBackend, style::Color, Terminal};
use std::fs;

fn fixture() -> String {
    serde_json::to_string(&Evidence {
        agent: None,
        candidate_commit: None,
        model_metrics: None, failure_code: None,
        generation: None,
        project_notes: None,
        run: "task-42-run-3".into(), baseline: "a".repeat(40), status: TaskStatus::Failed,
        detail: "Check did not run".into(),
        patch: "diff --git a/calc.py b/calc.py\n--- a/calc.py\n+++ b/calc.py\n@@ -1 +1 @@\n-old\n+new 界🦀\n\u{1b}[2J\u{7}tail".into(), check: None,
    }).unwrap()
}

#[test]
fn evidence_projection_preserves_diff_signs_and_never_infers_a_successful_check() {
    let view = View::from_verified(42, &fixture()).unwrap();
    let text = view
        .lines()
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("Task #42") && text.contains("No check receipt retained"));
    assert!(text.contains("\n-old\n+new 界🦀\n"));
    assert!(!text.contains('\\') && !text.contains('\u{1b}') && !text.contains('\u{7}'));
    assert_eq!(
        view.lines()
            .iter()
            .find(|l| l.to_string() == "-old")
            .unwrap()
            .style
            .fg,
        Some(Color::Red)
    );
    assert_eq!(
        view.lines()
            .iter()
            .find(|l| l.to_string() == "+new 界🦀")
            .unwrap()
            .style
            .fg,
        Some(Color::Green)
    );
    assert!(View::from_verified(42, "not json").is_err());
}

#[test]
fn evidence_view_remains_readable_and_scroll_clamps_after_resize() {
    let root = std::env::temp_dir().join(format!("alfredo-review-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let store = TaskStore::new(&root.join("state"), &workspace, "fixture").unwrap();
    let mut control = TaskControl::new(store);
    control.visible = true;
    control.evidence = Some(View::from_verified(42, &fixture()).unwrap());
    let app = App::new("fixture".into());
    for (width, height) in [(140, 40), (60, 25), (32, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        control.scroll = usize::MAX;
        terminal
            .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("tail"), "{width}x{height}: {text}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn evidence_page_keys_visit_every_row_forward_and_backward_after_resize() {
    let root = std::env::temp_dir().join(format!("alfredo-review-paging-{}", std::process::id()));
    fs::create_dir_all(root.join("workspace")).unwrap();
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "paging").unwrap();
    let snapshot = store.snapshot().unwrap();
    let mut control = TaskControl::new(store);
    control.snapshot = Some(snapshot);
    control.visible = true;
    let markers: Vec<_> = (0..40)
        .map(|index| format!("EVIDENCE_ROW_{index:02}"))
        .collect();
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["patch"] = markers.join("\n").into();
    control.evidence = Some(View::from_verified(42, &value.to_string()).unwrap());
    let app = App::new("fixture".into());
    let mut terminal = Terminal::new(TestBackend::new(32, 10)).unwrap();
    for (width, height) in [(32, 10), (140, 40), (32, 10)] {
        terminal.backend_mut().resize(width, height);
        terminal
            .resize(ratatui::layout::Rect::new(0, 0, width, height))
            .unwrap();
        control.scroll = 0;
        for forward in [true, false] {
            let mut seen = std::collections::BTreeSet::new();
            let mut reached_boundary = false;
            for _ in 0..256 {
                terminal
                    .draw(|frame| ui::draw_with_tasks(frame, &app, &control))
                    .unwrap();
                let screen: String = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect();
                for marker in &markers {
                    if screen.contains(marker) {
                        seen.insert(marker.clone());
                    }
                }
                let previous = control.scroll;
                control.page_details(forward);
                if control.scroll == previous {
                    reached_boundary = true;
                    break;
                }
            }
            assert!(reached_boundary, "paging must stop at the rendered bound");
            assert_eq!(
                seen.len(),
                markers.len(),
                "{width}x{height}, forward={forward}: skipped rows"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn generation_evidence_distinguishes_legacy_and_rejects_invalid_settings() {
    let legacy = fixture();
    assert!(View::from_verified(42, &legacy)
        .unwrap()
        .lines()
        .iter()
        .any(|line| line.to_string().contains("generation: unrecorded")));
    let mut value: serde_json::Value = serde_json::from_str(&legacy).unwrap();
    for mode in ["auto", "on", "off"] {
        value["generation"] =
            serde_json::json!({"thinking":mode,"num_predict":4096,"temperature":0});
        let raw = serde_json::to_string(&value).unwrap();
        let view = View::from_verified(42, &raw).unwrap();
        assert!(view
            .lines()
            .iter()
            .any(|line| line.to_string().contains(&format!("thinking {mode}"))));
        let evidence: Evidence = serde_json::from_str(&raw).unwrap();
        assert_eq!(evidence.detail, "Check did not run");
        assert_eq!(evidence.status, TaskStatus::Failed);
    }
    for invalid in [
        serde_json::json!({"thinking":"qualified","num_predict":4096,"temperature":0}),
        serde_json::json!({"thinking":"off","num_predict":0,"temperature":0}),
        serde_json::json!({"thinking":"off","num_predict":4096,"temperature":255}),
        serde_json::json!({"thinking":"off","num_predict":4096,"temperature":0,"authority":"approved"}),
    ] {
        value["generation"] = invalid;
        let raw = serde_json::to_string(&value).unwrap();
        assert!(serde_json::from_str::<Evidence>(&raw).is_err());
        assert!(View::from_verified(42, &raw).is_err());
    }
    let restored: Evidence = serde_json::from_str(&legacy).unwrap();
    assert!(restored.generation.is_none());
    assert!(serde_json::to_value(restored)
        .unwrap()
        .get("generation")
        .is_none());
}

#[test]
fn long_evidence_can_reach_its_last_line() {
    let root = std::env::temp_dir().join(format!("alfredo-long-review-{}", std::process::id()));
    fs::create_dir_all(root.join("workspace")).unwrap();
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "review").unwrap();
    let mut control = TaskControl::new(store);
    control.visible = true;
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["patch"] =
        serde_json::Value::String(format!("{}FINAL_EVIDENCE_LINE", "+x\n".repeat(66_000)));
    control.evidence = Some(View::from_verified(42, &value.to_string()).unwrap());
    control.scroll = usize::MAX;
    let app = App::new("test".into());
    let mut terminal = Terminal::new(TestBackend::new(88, 24)).unwrap();
    terminal
        .draw(|f| ui::draw_with_tasks(f, &app, &control))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        text.contains("FINAL_EVIDENCE_LINE"),
        "last evidence line is unreachable"
    );
    // Paging at the end must not accumulate an invisible offset. A single
    // page up must immediately move away from the end, including after resize.
    for _ in 0..100 {
        control.scroll_rows(10);
    }
    control.scroll_rows(-10);
    terminal
        .draw(|f| ui::draw_with_tasks(f, &app, &control))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(!text.contains("FINAL_EVIDENCE_LINE"));
    control.scroll_rows(100);
    terminal.backend_mut().resize(60, 18);
    terminal
        .resize(ratatui::layout::Rect::new(0, 0, 60, 18))
        .unwrap();
    terminal
        .draw(|f| ui::draw_with_tasks(f, &app, &control))
        .unwrap();
    control.scroll_rows(100);
    terminal
        .draw(|f| ui::draw_with_tasks(f, &app, &control))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("FINAL_EVIDENCE_LINE"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Explicit rendering benchmark; reports timing without a flaky wall-clock gate"]
fn retained_evidence_render_measurement() {
    let root = std::env::temp_dir().join(format!("alfredo-render-measure-{}", std::process::id()));
    fs::create_dir_all(root.join("workspace")).unwrap();
    let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "review").unwrap();
    let mut control = TaskControl::new(store);
    control.visible = true;
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["patch"] =
        serde_json::Value::String(format!("{}FINAL_EVIDENCE_LINE", "+x\n".repeat(66_000)));
    control.evidence = Some(View::from_verified(42, &value.to_string()).unwrap());
    control.scroll = usize::MAX;
    let mut app = App::new("test".into());
    let mut terminal = Terminal::new(TestBackend::new(88, 24)).unwrap();
    let started = std::time::Instant::now();
    terminal
        .draw(|f| ui::draw_with_tasks(f, &app, &control))
        .unwrap();
    let cold = started.elapsed();
    let started = std::time::Instant::now();
    for _ in 0..100 {
        app.sessions[0].insert("x");
        terminal
            .draw(|f| ui::draw_with_tasks(f, &app, &control))
            .unwrap();
    }
    println!(
        "render_evidence cold_us={} warm_100_us={}",
        cold.as_micros(),
        started.elapsed().as_micros()
    );
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("FINAL_EVIDENCE_LINE"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cached_evidence_pages_match_full_wrapping_styles_and_resize() {
    use ratatui::{
        layout::Rect,
        widgets::{Paragraph, Wrap},
    };
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["patch"] = serde_json::Value::String(
        "+界 🦀 e\u{301} wrapping text long enough for several rows\n-old styled line\n\n"
            .repeat(60),
    );
    let view = View::from_verified(42, &value.to_string()).unwrap();
    for (width, height) in [(30, 12), (58, 16), (86, 20), (30, 12)] {
        for offset in [0, 1, 11, 40, usize::MAX] {
            let page = view.page(width, height, offset);
            assert!(page.lines.len() <= height as usize);
            let mut actual = Terminal::new(TestBackend::new(width, height)).unwrap();
            actual
                .draw(|frame| {
                    frame.render_widget(
                        Paragraph::new(page.lines.clone())
                            .wrap(Wrap { trim: false })
                            .scroll((page.row, 0)),
                        Rect::new(0, 0, width, height),
                    )
                })
                .unwrap();
            let mut expected = Terminal::new(TestBackend::new(width, height)).unwrap();
            expected
                .draw(|frame| {
                    frame.render_widget(
                        Paragraph::new(view.lines().to_vec())
                            .wrap(Wrap { trim: false })
                            .scroll((offset.min(page.maximum) as u16, 0)),
                        Rect::new(0, 0, width, height),
                    )
                })
                .unwrap();
            assert_eq!(
                actual.backend().buffer(),
                expected.backend().buffer(),
                "{width}x{height} offset {offset}"
            );
        }
    }
}

#[test]
fn resolving_review_replaces_old_notes_without_losing_contract_or_dependency_inputs() {
    let view = View::from_verified(42, &fixture())
        .unwrap()
        .with_review(Some("Needs human review: inspect this"))
        .with_acceptance(&["Keep the expected calculation".into()])
        .with_inputs(&[alfredo_tui::tasks::DependencyInput {
            task: 3,
            source_task: None,
            run: "task-3-run-2".into(),
            evidence_sha256: "a".repeat(64),
            candidate: "b".repeat(40),
        }]);
    view.page(30, 12, 0);
    let resolved = view.with_review(Some(
        "Approved with limitations: reviewed\nLimitation: performance unmeasured",
    ));
    let text = resolved
        .lines()
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("Needs human review"));
    assert_eq!(text.matches("Recorded reviewer assessment").count(), 1);
    assert!(text.contains("Accepted dependency inputs: #3"));
    assert!(text.contains("1. Keep the expected calculation"));
    assert!(text.contains("+new 界🦀"));
    let first = resolved
        .page(80, 20, 0)
        .lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(first.contains("Limitation: performance unmeasured"));
}

fn current_task(status: TaskStatus, governed: bool, evidence: bool) -> alfredo_tui::tasks::Task {
    serde_json::from_value(serde_json::json!({
        "id": 42, "title": "t", "model": "m", "dependencies": [],
        "status": status,
        "policy": if governed { serde_json::json!({"files": ["calc.py"], "check": ["true"]}) } else { serde_json::Value::Null },
        "run": {"id": "task-42-run-3", "baseline": "a".repeat(40),
                "evidence_sha256": if evidence { serde_json::json!("c".repeat(64)) } else { serde_json::Value::Null },
                "detail": "d"},
    }))
    .unwrap()
}

fn status_and_hints(task: &alfredo_tui::tasks::Task) -> (String, String) {
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["status"] = serde_json::json!(TaskStatus::ReviewReady);
    value["detail"] = "Approved check passed; changes await human review".into();
    let view = View::from_verified_for(task, &value.to_string()).unwrap();
    let text: Vec<String> = view.lines().iter().map(|l| l.to_string()).collect();
    (text[1].clone(), text[4..].join("\n"))
}

#[test]
fn accepted_task_evidence_names_status_and_offers_no_review_commands() {
    let (detail, rest) = status_and_hints(&current_task(TaskStatus::Accepted, true, true));
    assert_eq!(
        detail,
        "Task #42 is accepted; recorded at run time: Approved check passed; changes await human review"
    );
    assert!(
        rest.contains("No review commands apply; the task is accepted."),
        "{rest}"
    );
    for gone in ["/accept", "/reject", "/repair", "/review"] {
        assert!(!rest.contains(gone), "{gone} offered for accepted: {rest}");
    }
}

#[test]
fn review_ready_task_keeps_stored_text_and_every_review_command() {
    let (detail, rest) = status_and_hints(&current_task(TaskStatus::ReviewReady, true, true));
    assert_eq!(detail, "Approved check passed; changes await human review");
    assert!(rest.contains("/review 42 JSON · /accept 42 · /reject 42 · /repair 42 reason"));
}

#[test]
fn terminal_unsuccessful_tasks_list_only_commands_that_would_succeed() {
    for (status, word) in [
        (TaskStatus::Failed, "failed"),
        (TaskStatus::Rejected, "rejected"),
        (TaskStatus::Cancelled, "cancelled"),
    ] {
        let (detail, rest) = status_and_hints(&current_task(status, true, true));
        assert_eq!(
            detail,
            format!("Task #42 is {word}; recorded at run time: Approved check passed; changes await human review")
        );
        assert!(rest.contains("/repair 42 reason"), "{rest}");
        for gone in ["/accept", "/reject", "/review"] {
            assert!(!rest.contains(gone), "{gone} offered: {rest}");
        }
    }
    for task in [
        current_task(TaskStatus::Failed, false, true),
        current_task(TaskStatus::Failed, true, false),
        current_task(TaskStatus::NeedsHumanReview, true, true),
    ] {
        let (_, rest) = status_and_hints(&task);
        for gone in ["/accept", "/reject", "/review", "/repair"] {
            assert!(!rest.contains(gone), "{gone} offered: {rest}");
        }
        assert!(rest.contains("No review commands apply"), "{rest}");
    }
}

#[test]
fn review_and_repair_eligibility_match_the_store_refusal_rules() {
    assert!(current_task(TaskStatus::ReviewReady, true, true).can_review());
    assert!(!current_task(TaskStatus::Accepted, true, true).can_review());
    assert!(current_task(TaskStatus::Cancelled, true, true).can_repair());
    assert!(!current_task(TaskStatus::Accepted, true, true).can_repair());
}

#[test]
fn refreshing_an_open_view_matches_a_view_built_for_the_new_status() {
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["status"] = serde_json::json!(TaskStatus::ReviewReady);
    value["detail"] = "Approved check passed; changes await human review".into();
    let raw = value.to_string();
    let lines =
        |view: &View| -> Vec<String> { view.lines().iter().map(|l| l.to_string()).collect() };
    for status in [
        TaskStatus::Accepted,
        TaskStatus::Rejected,
        TaskStatus::Cancelled,
        TaskStatus::ReviewReady,
    ] {
        let task = current_task(status, true, true);
        let open =
            View::from_verified_for(&current_task(TaskStatus::ReviewReady, true, true), &raw)
                .unwrap()
                .with_current(&task);
        assert_eq!(
            lines(&open),
            lines(&View::from_verified_for(&task, &raw).unwrap())
        );
        // Refreshing twice, or from an already refreshed view, keeps the stored detail.
        let again = open.with_current(&current_task(TaskStatus::Failed, true, true));
        assert_eq!(
            lines(&again),
            lines(
                &View::from_verified_for(&current_task(TaskStatus::Failed, true, true), &raw)
                    .unwrap()
            )
        );
    }
}

#[test]
fn refreshing_an_open_view_discards_the_cached_layout() {
    let mut value: serde_json::Value = serde_json::from_str(&fixture()).unwrap();
    value["status"] = serde_json::json!(TaskStatus::ReviewReady);
    value["detail"] = "Approved check passed; changes await human review".into();
    let raw = value.to_string();
    let view =
        View::from_verified_for(&current_task(TaskStatus::ReviewReady, true, true), &raw).unwrap();
    // Lay out at a narrow width first so a stale cache would hide the longer line.
    let before = view.page(20, 3, 0);
    assert!(before
        .lines
        .iter()
        .all(|l| !l.to_string().contains("is accepted")));
    let view = view.with_current(&current_task(TaskStatus::Accepted, true, true));
    let after = view.page(20, 3, 0);
    assert!(
        after
            .lines
            .iter()
            .any(|l| l.to_string().contains("Task #42 is accepted")),
        "{:?}",
        after
            .lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        after.maximum > before.maximum,
        "{} vs {}",
        after.maximum,
        before.maximum
    );
}
