//! The autopilot's failure and usage text, pinned with exact outputs. The store holds
//! real saved evidence (recovered through `TaskStore::recover`), not mocks.
use alfredo_tui::{
    autopilot::{
        failure::{
            check_cause, context_failure, failure_detail, held_or_done, interrupted_reason,
            last_failure_line, sandbox_failure, title_failure_line, too_large_failure,
            unreachable_failure, BUDGET_EXHAUSTED, INTERRUPTED,
        },
        state_path, Autopilot, Family, Settled,
    },
    execution::SANDBOX_UNAVAILABLE,
    metrics::{Metrics, Usage},
    task_control::TaskControl,
    tasks::{Action, Request, Task, TaskStatus, TaskStore, WorkPolicy},
    worker::{Evidence, CONTEXT_TOO_SMALL, FILE_TOO_LARGE, MODEL_UNREACHABLE},
};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tokio::runtime::Runtime;

static ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    store: TaskStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "alfredo-failure-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("workspace")).unwrap();
        let store = TaskStore::new(&root.join("state"), &root.join("workspace"), "test").unwrap();
        Self { root, store }
    }
    fn transact(&self, action: Action) {
        let revision = self.store.snapshot().unwrap().revision;
        self.store
            .transact(Request {
                correlation: format!("failure-{revision}"),
                expected_revision: revision,
                action,
            })
            .unwrap();
    }
    /// Propose, approve, run and recover one task so it is `Failed` with acknowledged
    /// evidence carrying `code`, `detail` and `metrics`.
    fn fail(&self, task: u64, code: Option<&str>, detail: &str, metrics: Option<Metrics>) -> Task {
        self.transact(Action::Propose {
            title: format!("task {task}"),
            model: "fixture".into(),
            dependencies: vec![],
        });
        self.transact(Action::Permit {
            task,
            policy: WorkPolicy {
                files: vec!["file".into()],
                check: vec!["/bin/true".into()],
            },
        });
        self.transact(Action::Approve { task });
        let owner = self.store.claim_worker(task).unwrap();
        self.transact(Action::Start {
            inputs: vec![],
            task,
            baseline: "a".repeat(40),
        });
        let run = self
            .store
            .snapshot()
            .unwrap()
            .tasks
            .iter()
            .find(|item| item.id == task)
            .and_then(|item| item.run.clone())
            .unwrap();
        let directory = self.store.run_directory(&run.id).unwrap();
        fs::create_dir_all(&directory).unwrap();
        let evidence = Evidence {
            agent: None,
            candidate_commit: None,
            model_metrics: metrics,
            failure_code: code.map(str::to_string),
            generation: None,
            project_notes: None,
            run: run.id.clone(),
            baseline: "a".repeat(40),
            status: TaskStatus::Failed,
            detail: detail.into(),
            patch: String::new(),
            check: None,
        };
        fs::write(
            directory.join("evidence.json"),
            serde_json::to_vec(&evidence).unwrap(),
        )
        .unwrap();
        drop(owner);
        self.store.recover(task).unwrap();
        self.store
            .snapshot()
            .unwrap()
            .tasks
            .into_iter()
            .find(|item| item.id == task)
            .unwrap()
    }
    fn control(&self, runtime: &Runtime) -> TaskControl {
        let mut control = TaskControl::new(self.store.clone());
        control.refresh(runtime);
        let deadline = Instant::now() + Duration::from_secs(5);
        while control.pending || control.snapshot.is_none() {
            control.poll();
            assert!(Instant::now() < deadline, "{}", control.notice);
            std::thread::sleep(Duration::from_millis(2));
        }
        control
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_typed_environment_failures_each_quote_only_their_own_cause() {
    let long = "x".repeat(400);
    for (code, expected) in [
        (SANDBOX_UNAVAILABLE, "sandbox"),
        (FILE_TOO_LARGE, "large"),
        (MODEL_UNREACHABLE, "unreachable"),
        (CONTEXT_TOO_SMALL, "context"),
    ] {
        let fixture = Fixture::new();
        let runtime = Runtime::new().unwrap();
        let head = fixture.fail(1, Some(code), "bwrap refused the namespace", None);
        let control = fixture.control(&runtime);
        let causes = [
            ("sandbox", sandbox_failure(&control, &head)),
            ("large", too_large_failure(&control, &head)),
            ("unreachable", unreachable_failure(&control, &head)),
            ("context", context_failure(&control, &head)),
        ];
        for (name, cause) in causes {
            let want = (name == expected).then(|| "bwrap refused the namespace".to_string());
            assert_eq!(cause, want, "code {code} asked as {name}");
        }
    }
    // The cause is bounded to 300 bytes.
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let head = fixture.fail(1, Some(SANDBOX_UNAVAILABLE), &long, None);
    let control = fixture.control(&runtime);
    assert_eq!(sandbox_failure(&control, &head), Some("x".repeat(300)));
}

#[test]
fn a_failed_head_without_a_typed_code_is_not_an_environment_failure() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let head = fixture.fail(1, None, "check failed", None);
    let control = fixture.control(&runtime);
    assert_eq!(sandbox_failure(&control, &head), None);
    assert_eq!(too_large_failure(&control, &head), None);
    assert_eq!(unreachable_failure(&control, &head), None);
    assert_eq!(context_failure(&control, &head), None);
}

#[test]
fn only_a_failed_head_is_an_environment_failure_whatever_its_saved_evidence_says() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let mut head = fixture.fail(1, Some(SANDBOX_UNAVAILABLE), "no namespaces", None);
    let control = fixture.control(&runtime);
    assert_eq!(
        sandbox_failure(&control, &head),
        Some("no namespaces".into())
    );
    head.status = TaskStatus::Cancelled;
    assert_eq!(sandbox_failure(&control, &head), None);
    head.status = TaskStatus::Failed;
    let mut large = head.clone();
    large.status = TaskStatus::Cancelled;
    assert_eq!(too_large_failure(&control, &large), None);
    assert_eq!(unreachable_failure(&control, &large), None);
}

#[test]
fn the_environment_failures_of_other_codes_read_their_own_evidence() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let large = fixture.fail(1, Some(FILE_TOO_LARGE), "big.py is 9 MiB", None);
    let down = fixture.fail(2, Some(MODEL_UNREACHABLE), "Cannot reach Ollama", None);
    let mut small = fixture.fail(3, Some(CONTEXT_TOO_SMALL), "prompt too big", None);
    let control = fixture.control(&runtime);
    assert_eq!(
        too_large_failure(&control, &large),
        Some("big.py is 9 MiB".into())
    );
    assert_eq!(
        unreachable_failure(&control, &down),
        Some("Cannot reach Ollama".into())
    );
    assert_eq!(
        context_failure(&control, &small),
        Some("prompt too big".into())
    );
    assert_eq!(context_failure(&control, &large), None);
    assert_eq!(context_failure(&control, &down), None);
    small.status = TaskStatus::Cancelled;
    assert_eq!(context_failure(&control, &small), None);
    // A head whose run is absent from the store reads no evidence.
    let mut stranger = large.clone();
    stranger.id = 99;
    assert_eq!(too_large_failure(&control, &stranger), None);
}

#[test]
fn an_interrupted_run_says_why_it_cannot_be_recovered() {
    let fixture = Fixture::new();
    let mut control = TaskControl::new(fixture.store.clone());
    control.run_observations.insert(
        1,
        "owner active: its worker is held by another process".into(),
    );
    control
        .run_observations
        .insert(2, format!("stopped after check\t{}", "y".repeat(300)));
    assert_eq!(INTERRUPTED, "run interrupted");
    assert_eq!(BUDGET_EXHAUSTED, "repair budget exhausted");
    assert_eq!(
        interrupted_reason(&control, 1),
        "run interrupted: its worker is still active in another Alfredo process"
    );
    assert_eq!(
        interrupted_reason(&control, 2),
        format!(
            "run interrupted: it cannot be recovered safely (stopped after check {})",
            "y".repeat(200 - "stopped after check ".len())
        )
    );
    assert_eq!(
        interrupted_reason(&control, 3),
        "run interrupted: its worker state is unknown"
    );
}

#[test]
fn the_last_failure_line_is_the_first_nonblank_line_shortened_on_a_character() {
    assert_eq!(last_failure_line(""), "no detail recorded");
    assert_eq!(last_failure_line(" \n\t\n"), "no detail recorded");
    assert_eq!(
        last_failure_line("  \n\n   hello\tworld  \nsecond"),
        "hello world"
    );
    let exact = "a".repeat(160);
    assert_eq!(last_failure_line(&exact), exact);
    assert_eq!(
        last_failure_line(&"a".repeat(161)),
        format!("{}…", "a".repeat(159))
    );
    assert_eq!(
        last_failure_line(&"é".repeat(170)),
        format!("{}…", "é".repeat(159))
    );
}

#[test]
fn a_check_cause_drops_the_stream_label_and_ends_with_the_key_error_line() {
    for (detail, cause) in [
        (
            "Check failed (exit 1): stderr: Traceback (most recent call last) | File x | AssertionError",
            "Check failed (exit 1): AssertionError",
        ),
        (
            "Check failed (exit 1): pytest -q: stderr: setup | AssertionError: 41 != 42 | done",
            "Check failed (exit 1): pytest -q: AssertionError: 41 != 42",
        ),
        (
            "Check failed (exit 1): t · stdout: a | FAILED z",
            "Check failed (exit 1): t: FAILED z",
        ),
    ] {
        assert_eq!(check_cause(detail, 100).as_deref(), Some(cause), "{detail}");
    }
}

#[test]
fn a_check_cause_cuts_the_head_first_and_then_the_key_line_at_the_cell_limit() {
    let detail = format!(
        "Check failed (exit 1): {}: stderr: x | AssertionError: nope",
        "cmd ".repeat(20)
    );
    let cause = check_cause(&detail, 40).unwrap();
    assert!(cause.ends_with(": AssertionError: nope"), "{cause}");
    assert!(
        cause.contains('…') && cause.chars().count() <= 40,
        "{cause}"
    );
    // No room for a head: the key line alone, cut with an ellipsis.
    assert_eq!(
        check_cause("Check failed (exit 1): stderr: AssertionError: nope", 10).as_deref(),
        Some("Assertion…")
    );
}

#[test]
fn a_check_cause_is_none_without_check_output_to_pick_from() {
    assert_eq!(check_cause("Check failed (exit 1)", 80), None);
    assert_eq!(check_cause("Worker cancelled after check", 80), None);
}

#[test]
fn the_footer_text_of_a_failed_run_is_its_check_cause_and_others_are_unchanged() {
    use alfredo_tui::worker::finish_notice;
    assert_eq!(
        finish_notice("Check failed (exit 1): stderr: Traceback (most recent call last): | File \"<string>\", line 1, in <module> | AssertionError"),
        "Check failed (exit 1): AssertionError"
    );
    assert_eq!(
        finish_notice("Approved check passed; changes await human review"),
        "Approved check passed; changes await human review"
    );
    assert_eq!(
        finish_notice("Check failed (exit 1): stderr: a\u{202e}b | Error: x\u{200b}y"),
        "Check failed (exit 1): Error: x<U+200B>y"
    );
}

#[test]
fn a_failed_check_keeps_its_head_and_ends_with_the_key_error_line() {
    // The error line is picked out of the output.
    assert_eq!(
        last_failure_line(
            "Check failed (exit 1): pytest -q: stderr: setup | AssertionError: 41 != 42 | done"
        ),
        "Check failed (exit 1): pytest -q: stderr: AssertionError: 41 != 42"
    );
    // Nothing recognisable: the last output line.
    assert_eq!(
        last_failure_line("Check failed (exit 2): make: stderr: compiling | linking | done"),
        "Check failed (exit 2): make: stderr: done"
    );
    // The earliest of the stream labels ends the head.
    assert_eq!(
        last_failure_line("Check failed (exit 1): t: stdout: a | b · stderr: z"),
        "Check failed (exit 1): t: stdout: b · stderr: z"
    );
    assert_eq!(
        last_failure_line("Check failed (exit 1): t · stderr: a | FAILED z"),
        "Check failed (exit 1): t · stderr: FAILED z"
    );
    assert_eq!(
        last_failure_line("Check failed (exit 1): t · stdout: a | FAILED z"),
        "Check failed (exit 1): t · stdout: FAILED z"
    );
    // An elided output start is ignored.
    assert_eq!(
        last_failure_line("Check failed (exit 1): t: stderr: …a | FAILED z"),
        "Check failed (exit 1): t: stderr: FAILED z"
    );
    // The check head need not start the line.
    assert_eq!(
        last_failure_line("note: Check failed (exit 1): t: stderr: FAILED z"),
        "note: Check failed (exit 1): t: stderr: FAILED z"
    );
    // A label without a check head is plain text.
    assert_eq!(
        last_failure_line("failed: stderr: a | FAILED z"),
        "failed: stderr: a | FAILED z"
    );
    // A check without output has no key line.
    assert_eq!(
        last_failure_line("Check failed (exit 1): t: stderr: "),
        "Check failed (exit 1): t: stderr:"
    );
}

#[test]
fn the_key_line_skips_the_omitted_lines_note() {
    let failing: Vec<String> = (1..=31).map(|n| format!("FAILED t{n:02}")).collect();
    let line = format!("Check failed (exit 1): t: stderr: {}", failing.join(" | "));
    assert_eq!(
        last_failure_line(&line),
        "Check failed (exit 1): t: stderr: FAILED t30"
    );
}

#[test]
fn the_head_is_cut_before_the_key_line_and_the_whole_fits_in_160_characters() {
    let tail = ": stderr: FAILED z";
    // Head of exactly the room left (160 - 8 - 1 = 151): kept whole.
    let fits = format!("Check failed (exit 1): {}", "p".repeat(119));
    let line = last_failure_line(&format!("{fits}{tail}"));
    assert_eq!(line, format!("{fits}: stderr: FAILED z"));
    assert_eq!(line.chars().count(), 160);
    // One more character and the head gives way.
    let over = format!("Check failed (exit 1): {}", "p".repeat(120));
    let line = last_failure_line(&format!("{over}{tail}"));
    let head = format!("{over}: stderr:");
    assert_eq!(
        line,
        format!("{}… FAILED z", head.chars().take(150).collect::<String>())
    );
}

#[test]
fn a_long_key_line_is_cut_to_120_characters_and_leaves_the_rest_to_the_head() {
    let line = last_failure_line(&format!(
        "Check failed (exit 1): t: stderr: FAILED {}",
        "k".repeat(300)
    ));
    let key = format!("FAILED {}…", "k".repeat(112));
    assert_eq!(key.chars().count(), 120);
    assert_eq!(line, format!("Check failed (exit 1): t: stderr: {key}"));
    // Past the key's share the head is shortened to what is left of 160.
    let long_head = format!("Check failed (exit 1): {}", "h".repeat(100));
    let line = last_failure_line(&format!("{long_head}: stderr: FAILED {}", "k".repeat(300)));
    let room = 160 - 120 - 1;
    let head: String = long_head.chars().take(room - 1).collect();
    assert_eq!(line, format!("{head}… {key}"));
    assert_eq!(line.chars().count(), 160);
}

#[test]
fn a_repair_title_shows_the_same_summary_line_as_last_failure() {
    assert_eq!(
        title_failure_line(
            "Repair #3: autopilot: Check failed (exit 1): t: stderr: x | AssertionError: no"
        ),
        Some("Check failed (exit 1): t: stderr: AssertionError: no".into())
    );
    assert_eq!(
        title_failure_line("Repair #3: autopilot: a: autopilot: b"),
        Some("a: autopilot: b".into())
    );
    assert_eq!(title_failure_line("Repair #3: something else"), None);
    assert_eq!(title_failure_line("autopilot: no repair prefix"), None);
}

#[test]
fn the_repair_reason_detail_is_the_recorded_run_detail_and_never_empty() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let head = fixture.fail(1, None, "Model failed before applying any files", None);
    let control = fixture.control(&runtime);
    assert_eq!(
        failure_detail(&control, &head),
        head.run.as_ref().unwrap().detail
    );
    assert!(!head.run.as_ref().unwrap().detail.is_empty());
    for blank in ["", "   ", ":", " : ", "::"] {
        let mut bare = head.clone();
        bare.run.as_mut().unwrap().detail = blank.into();
        assert_eq!(
            failure_detail(&control, &bare),
            "worker failed; no detail recorded",
            "{blank:?}"
        );
    }
    let mut colon = head.clone();
    colon.run.as_mut().unwrap().detail = "Failed:".into();
    assert_eq!(failure_detail(&control, &colon), "Failed:");
    let mut missing = head.clone();
    missing.id = 77;
    missing.run.as_mut().unwrap().detail = "  ::  ".into();
    assert_eq!(
        failure_detail(&control, &missing),
        "worker failed; no detail recorded"
    );
    missing.run = None;
    assert_eq!(
        failure_detail(&control, &missing),
        "worker failed; no detail recorded"
    );
    missing.run = head.run.clone();
    missing.run.as_mut().unwrap().detail = format!("{}\n", "d".repeat(2000));
    assert_eq!(failure_detail(&control, &missing), "d".repeat(1780));
}

#[test]
fn run_usage_adds_only_this_runs_acknowledged_task_metrics() {
    let fixture = Fixture::new();
    let runtime = Runtime::new().unwrap();
    let metrics = |prompt, eval, total| Metrics {
        total_duration: Some(total),
        load_duration: None,
        prompt_eval_duration: None,
        eval_duration: None,
        prompt_eval_count: Some(prompt),
        eval_count: Some(eval),
    };
    // Task 1 predates the run, task 2 is the run's, task 3 is an unrelated task.
    fixture.fail(1, None, "old", Some(metrics(1000, 2000, 3000)));
    fixture.fail(2, None, "run", Some(metrics(100, 20, 5)));
    fixture.fail(3, None, "other", Some(metrics(7, 3, 1)));
    let directory = fixture.root.join("autopilot");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        state_path(&directory, "default"),
        serde_json::to_vec(&json!({
            "version": 1, "id": "0123456789abcdef", "goal": "g", "model": "fixture",
            "max_repairs": 3, "phase": "running", "paused": true, "started": 0,
            "first": 2, "count": 1,
        }))
        .unwrap(),
    )
    .unwrap();
    let autopilot = Autopilot::open(&directory, "default").unwrap();
    let control = fixture.control(&runtime);
    assert_eq!(
        autopilot.usage(&control),
        Usage {
            prompt_tokens: Some(100),
            completion_tokens: Some(20),
            model_ns: Some(5),
        }
    );
}

fn family(root: u64, settled: Settled) -> Family {
    Family {
        root,
        head: root,
        settled,
        repairs: 0,
    }
}

/// A control whose owner holds an active instruction on `held_root`, if any.
fn owner_holding(fixture: &Fixture, held_root: Option<u64>) -> TaskControl {
    let mut tasks = TaskControl::new(fixture.store.clone());
    if let Some(held) = held_root {
        let item = json!({
            "id": 1, "root": held, "task": held, "note": "n", "effect": "repair",
            "status": "queued", "done": false,
        });
        let state = json!({"version": 1, "next": 2, "instructions": [item]});
        fs::write(
            alfredo_tui::instruct::state_path(&fixture.root, "c"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        tasks.owner = alfredo_tui::instruct::Instructions::open(&fixture.root, "c").unwrap();
    }
    tasks
}

#[test]
fn a_family_is_held_or_done_when_it_succeeded_or_the_owner_holds_it() {
    let fixture = Fixture::new();
    let free = owner_holding(&fixture, None);
    let held = owner_holding(&fixture, Some(4));
    assert!(held_or_done(&family(4, Settled::Active), &held));
    assert!(!held_or_done(&family(5, Settled::Active), &held));
    assert!(held_or_done(&family(4, Settled::Success(9)), &free));
    assert!(!held_or_done(&family(4, Settled::Active), &free));
    assert!(!held_or_done(&family(4, Settled::Stuck("x".into())), &free));
    assert!(!held_or_done(&family(4, Settled::Cancelled), &free));
}
