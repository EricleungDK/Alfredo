//! Agent view projection: pure turns from retained conversation, evidence,
//! live worker state and owner notes. No rendering, no files.
use alfredo_tui::{
    agent_view::{self, Attempt, Check, Live, Note, Origin, Recorded, Tone, Turn},
    tasks::TaskStatus,
};

fn attempt(id: u64, status: TaskStatus) -> Attempt {
    Attempt {
        id,
        title: "Create greet.py and test_greet.py".into(),
        status,
        repair_of: None,
        origin: Origin::Autopilot,
        files: vec!["greet.py".into(), "test_greet.py".into()],
        check: vec![
            "python3".into(),
            "-m".into(),
            "unittest".into(),
            "test_greet.py".into(),
        ],
        recorded: None,
        live: None,
    }
}

fn prompt() -> String {
    "Implement this task: Create greet.py and test_greet.py\nAllowed exact files: [\"greet.py\"]\n\nFILE greet.py\n(new file)\n\nREAD-ONLY REFERENCE FILES (committed baseline; reference only)\nREAD-ONLY FILE README.md\nhello\nREAD-ONLY FILE docs/usage.md\nuse it\n".into()
}

fn answer() -> String {
    "=== FILE: greet.py ===\ndef greet(name):\n    return f\"Hello, {name}!\"\n=== END FILE ===\n=== FILE: test_greet.py ===\nimport unittest\n=== END FILE ===\n".into()
}

fn recorded(passed: bool) -> Recorded {
    Recorded {
        prompt: Some(prompt()),
        answer: Some(answer()),
        check: Some(Check {
            passed,
            exit: Some(if passed { 0 } else { 1 }),
            tail: if passed {
                "Ran 1 test in 0.001s\nOK".into()
            } else {
                "AssertionError: 'Hi' != 'Hello'\nFAILED (failures=1)".into()
            },
        }),
        detail: if passed {
            "Approved check passed; changes await human review".into()
        } else {
            "Check completed (exit 1): stderr: FAILED".into()
        },
        note: None,
        cut: None,
    }
}

fn labels(turns: &[Turn]) -> Vec<&str> {
    turns.iter().map(|turn| turn.label.as_str()).collect()
}

fn text(turn: &Turn) -> Vec<&str> {
    turn.lines.iter().map(|(text, _)| text.as_str()).collect()
}

#[test]
fn finished_attempt_reads_as_instruction_references_code_check_and_outcome() {
    let mut first = attempt(1, TaskStatus::Accepted);
    first.recorded = Some(Ok(recorded(true)));
    let turns = agent_view::project(&[first], &[], false);
    assert_eq!(
        labels(&turns),
        [
            "Autopilot → worker #1",
            "References",
            "Worker",
            "Check",
            "Outcome"
        ]
    );
    // Collapsed instruction: title plus one policy line.
    assert_eq!(
        text(&turns[0]),
        [
            "Create greet.py and test_greet.py",
            "files greet.py, test_greet.py · check python3 -m unittest test_greet.py"
        ]
    );
    assert_eq!(turns[0].lines[1].1, Tone::Summary);
    assert_eq!(text(&turns[1]), ["README.md, docs/usage.md"]);
    // FILE blocks read as code per file with the existing display rules.
    assert_eq!(
        text(&turns[2]),
        [
            "▸ greet.py",
            "def greet(name):",
            "    return f\"Hello, {name}!\"",
            "▸ test_greet.py",
            "import unittest"
        ]
    );
    assert_eq!(turns[2].lines[0].1, Tone::Path);
    assert_eq!(
        turns[3].lines[0],
        ("python3 -m unittest test_greet.py".into(), Tone::Dim)
    );
    assert_eq!(
        turns[3].lines.last().unwrap(),
        &("✓ passed · exit 0".into(), Tone::Pass)
    );
    assert_eq!(text(&turns[4]), ["✓ Accepted"]);
    assert_eq!(turns[4].lines[0].1, Tone::Pass);
}

#[test]
fn expanded_instruction_shows_the_request_text() {
    let mut first = attempt(1, TaskStatus::Accepted);
    first.recorded = Some(Ok(recorded(true)));
    let turns = agent_view::project(&[first], &[], true);
    let lines = text(&turns[0]);
    assert_eq!(lines[0], "Create greet.py and test_greet.py");
    assert!(lines.contains(&"Implement this task: Create greet.py and test_greet.py"));
    assert!(lines.contains(&"READ-ONLY FILE docs/usage.md"));
}

#[test]
fn repair_attempts_and_owner_notes_follow_in_order() {
    let mut first = attempt(1, TaskStatus::Failed);
    first.recorded = Some(Ok(recorded(false)));
    let mut repair = attempt(3, TaskStatus::Running);
    repair.title = "Repair #1: Owner: greet must say Hello".into();
    repair.repair_of = Some(1);
    repair.origin = Origin::You;
    repair.live = Some(Live {
        stage: "generating".into(),
        prompt: Some(prompt()),
        output: "=== FILE: greet.py ===\ndef greet(name):\n".into(),
        check_output: String::new(),
        checking: false,
        cancelling: false,
    });
    let notes = [Note {
        task: 1,
        text: "greet must say Hello".into(),
        status: "repair #3 started".into(),
    }];
    let turns = agent_view::project(&[first, repair], &notes, false);
    assert_eq!(
        labels(&turns),
        [
            "Autopilot → worker #1",
            "References",
            "Worker",
            "Check",
            "Outcome",
            "You",
            "You → repair #3",
            "References",
            "Worker"
        ]
    );
    assert_eq!(
        turns[3].lines.last().unwrap(),
        &("✗ failed · exit 1".into(), Tone::Fail)
    );
    assert!(text(&turns[3]).contains(&"AssertionError: 'Hi' != 'Hello'"));
    assert_eq!(turns[4].lines[0].1, Tone::Fail);
    assert_eq!(
        turns[5].lines,
        vec![
            ("greet must say Hello".into(), Tone::Normal),
            ("repair #3 started".into(), Tone::Dim)
        ]
    );
    // The repair made from the note names it rather than repeating it.
    assert_eq!(text(&turns[6])[0], "Repair #1 with your note");
    // A live attempt streams its answer; no outcome yet.
    assert_eq!(text(&turns[8]), ["▸ greet.py", "def greet(name):"]);
}

#[test]
fn live_check_shows_the_command_and_streamed_output() {
    let mut first = attempt(2, TaskStatus::Running);
    first.live = Some(Live {
        stage: "check".into(),
        prompt: None,
        output: answer(),
        check_output: "Ran 1 test\n".into(),
        checking: true,
        cancelling: false,
    });
    let turns = agent_view::project(&[first], &[], false);
    assert_eq!(labels(&turns), ["Autopilot → worker #2", "Worker", "Check"]);
    assert_eq!(
        text(&turns[2]),
        [
            "python3 -m unittest test_greet.py",
            "Ran 1 test",
            "… running"
        ]
    );
    assert_eq!(turns[2].lines[2].1, Tone::Warn);
}

#[test]
fn waiting_worker_says_what_it_is_doing_without_inventing_output() {
    let mut first = attempt(2, TaskStatus::Running);
    first.live = Some(Live {
        stage: "queued".into(),
        prompt: None,
        output: String::new(),
        check_output: String::new(),
        checking: false,
        cancelling: true,
    });
    let turns = agent_view::project(&[first], &[], false);
    assert_eq!(labels(&turns), ["Autopilot → worker #2", "Worker"]);
    assert_eq!(
        turns[1].lines,
        vec![("… queued · cancelling".into(), Tone::Warn)]
    );
}

#[test]
fn missing_or_corrupt_conversation_shows_evidence_with_a_note_and_never_fabricates() {
    // Legacy run: evidence without a retained conversation.
    let mut legacy = attempt(1, TaskStatus::Failed);
    legacy.recorded = Some(Ok(Recorded {
        prompt: None,
        answer: None,
        check: None,
        detail: "Model returned no FILE blocks".into(),
        note: Some("No retained conversation for this run".into()),
        cut: None,
    }));
    let turns = agent_view::project(&[legacy], &[], true);
    assert_eq!(labels(&turns), ["Autopilot → worker #1", "Outcome"]);
    assert_eq!(
        turns[1].lines,
        vec![
            (
                "✗ Failed · Model returned no FILE blocks".into(),
                Tone::Fail
            ),
            ("No retained conversation for this run".into(), Tone::Dim)
        ]
    );
    // The expanded instruction admits the request text is not retained.
    assert!(text(&turns[0]).contains(&"Request text not retained"));
    // Unreadable evidence: status only, with the reason.
    let mut corrupt = attempt(1, TaskStatus::Failed);
    corrupt.recorded = Some(Err("Evidence digest mismatch".into()));
    let turns = agent_view::project(&[corrupt], &[], false);
    assert_eq!(labels(&turns), ["Autopilot → worker #1", "Outcome"]);
    assert_eq!(
        turns[1].lines,
        vec![
            ("✗ Failed".into(), Tone::Fail),
            (
                "Evidence unavailable: Evidence digest mismatch".into(),
                Tone::Dim
            )
        ]
    );
    // Not started: only the instruction.
    let turns = agent_view::project(&[attempt(4, TaskStatus::Approved)], &[], false);
    assert_eq!(labels(&turns), ["Autopilot → worker #4"]);
}

#[test]
fn titles_name_the_current_attempt() {
    let first = attempt(2, TaskStatus::Running);
    assert_eq!(agent_view::title(&[first]), "Agent · worker #2 · running");
    let mut repair = attempt(3, TaskStatus::Failed);
    repair.repair_of = Some(2);
    assert_eq!(
        agent_view::title(&[attempt(2, TaskStatus::Failed), repair]),
        "Agent · repair #3 of #2 · failed"
    );
    assert_eq!(agent_view::title(&[]), "Agent");
}

#[test]
fn worker_turn_hides_markdown_fences_but_keeps_code_under_path_headings() {
    let mut first = attempt(1, TaskStatus::Failed);
    let mut rec = recorded(false);
    rec.answer = Some(
        "Sure:\n=== FILE: greet.py ===\n```python\ndef greet(name):\n    return name\n```\n=== END FILE ===\n"
            .into(),
    );
    first.recorded = Some(Ok(rec));
    let turns = agent_view::project(&[first], &[], false);
    let worker = turns.iter().find(|turn| turn.label == "Worker").unwrap();
    assert_eq!(
        text(worker),
        ["Sure:", "▸ greet.py", "def greet(name):", "    return name"]
    );
    // A live partial fence line is hidden until it is complete.
    let mut running = attempt(2, TaskStatus::Running);
    running.live = Some(Live {
        stage: "generating".into(),
        prompt: None,
        output: "=== FILE: greet.py ===\n```pyth".into(),
        check_output: String::new(),
        checking: false,
        cancelling: false,
    });
    let turns = agent_view::project(&[running], &[], false);
    let worker = turns.iter().find(|turn| turn.label == "Worker").unwrap();
    assert_eq!(text(worker), ["▸ greet.py"]);
}

#[test]
fn steered_attempt_shows_partial_output_then_a_dim_cut_marker() {
    let mut steered = attempt(1, TaskStatus::Cancelled);
    let mut rec = recorded(false);
    rec.check = None;
    rec.detail = "Worker cancelled during inference".into();
    rec.answer = Some("=== FILE: greet.py ===\n```python\ndef greet(name):\n    ret".into());
    rec.cut = Some(alfredo_tui::agent::Cut {
        elapsed_secs: Some(12),
    });
    steered.recorded = Some(Ok(rec.clone()));
    let turns = agent_view::project(&[steered.clone()], &[], false);
    assert_eq!(
        labels(&turns),
        ["Autopilot → worker #1", "References", "Worker", "Outcome"]
    );
    assert_eq!(
        text(&turns[2]),
        [
            "▸ greet.py",
            "def greet(name):",
            "    ret",
            "— steered at 12s · output cut"
        ]
    );
    assert_eq!(turns[2].lines.last().unwrap().1, Tone::Dim);
    assert_eq!(turns[2].lines[0].1, Tone::Path);
    // The outcome still follows.
    assert!(text(&turns[3])[0].starts_with("– Cancelled"));
    // Elapsed unavailable: no invented number.
    rec.cut = Some(alfredo_tui::agent::Cut { elapsed_secs: None });
    steered.recorded = Some(Ok(rec.clone()));
    let turns = agent_view::project(&[steered.clone()], &[], false);
    assert_eq!(
        text(&turns[2]).last().unwrap(),
        &"— steered · output cut"
    );
    // Nothing streamed: no worker turn and no marker to fabricate.
    rec.answer = Some(String::new());
    steered.recorded = Some(Ok(rec.clone()));
    let turns = agent_view::project(&[steered.clone()], &[], false);
    assert!(!labels(&turns).contains(&"Worker"));
    // Output hidden by the display rules only (a lone fence) is not shown either.
    rec.answer = Some("```".into());
    steered.recorded = Some(Ok(rec));
    let turns = agent_view::project(&[steered], &[], false);
    assert!(!labels(&turns).contains(&"Worker"));
}
