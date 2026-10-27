//! Issue #187: summarising compaction replaces the oldest turns instead of hard-stopping.
use alfredo_tui::model::{
    compaction::{self, Outcome, HARD_CAP},
    App, Session, Status, Update,
};

const PASTE: usize = 15_000;

/// A finished turn: `prompt` is a paste of `PASTE` bytes starting with the turn number.
fn turn(session: &mut Session, number: usize) {
    session.insert(&format!("turn{number} {}", "x".repeat(PASTE - 10)));
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Token(format!("reply{number}")));
    session.apply(attempt, Update::Done);
}

fn long(turns: usize) -> Session {
    let mut session = Session::new("fixture".into());
    for number in 1..=turns {
        turn(&mut session, number);
    }
    session
}

#[test]
fn enter_or_compaction_during_a_turn_names_the_esc_key() {
    let mut session = Session::new("fixture".into());
    session.insert("first");
    session.begin().unwrap();
    session.insert("second");
    assert_eq!(
        session.begin().unwrap_err(),
        "Esc stops the reply, or wait for it"
    );
    assert_eq!(
        session.start_compaction(None, true).unwrap_err(),
        "Esc stops the reply, or wait for it"
    );
}

fn compact(session: &mut Session, summary: &str) -> Outcome {
    let plan = session.start_compaction(None, true).unwrap();
    let attempt = session.attempt;
    session
        .finish_compaction(attempt, &plan, Ok(summary.into()))
        .unwrap()
}

#[test]
fn compaction_triggers_before_the_hard_limit_and_not_before_the_budget() {
    let mut session = long(5);
    session.insert(&"y".repeat(PASTE));
    assert!(
        !session.compaction_due(None),
        "5 turns + prompt = 90 KB is under the budget"
    );
    let mut session = long(6);
    session.insert(&"y".repeat(PASTE));
    assert!(
        session.compaction_due(None),
        "6 turns + prompt passes 96 KiB"
    );
    assert!(session.begin().is_ok(), "still below the 128 KiB hard cap");
}

#[test]
fn nothing_is_due_for_an_empty_prompt_a_short_history_or_an_active_reply() {
    let mut session = long(6);
    assert!(!session.compaction_due(None), "no prompt");
    session.insert("next");
    session.begin().unwrap();
    session.insert("another");
    assert!(!session.compaction_due(None), "reply still streaming");
    assert!(!Session::new("m".into()).compaction_due(None));
}

#[test]
fn num_ctx_lowers_the_threshold() {
    let mut session = long(2);
    session.insert("next question");
    assert!(!session.compaction_due(None));
    assert!(
        session.compaction_due(Some(2048)),
        "15 KB does not fit a 2048-token window"
    );
    assert_eq!(compaction::budget(Some(262_144)), compaction::COMPACT_AT);
}

#[test]
fn oldest_turns_are_replaced_and_newest_stay_verbatim() {
    let mut session = long(7);
    let newest: Vec<_> = session.messages[10..].to_vec();
    session.insert("the new prompt");
    let plan = session.start_compaction(None, true).unwrap();
    assert_eq!(plan.turns, 5);
    // The summary input holds exactly the replaced turns, never the kept ones.
    let sent: String = plan.transcript.iter().map(|m| m.content.as_str()).collect();
    assert!(sent.contains("turn1 ") && sent.contains("reply5"));
    assert!(!sent.contains("turn6") && !sent.contains("turn7"));
    assert!(session.compacting());
    assert!(
        session.begin().is_err(),
        "the prompt waits while summarising"
    );
    let attempt = session.attempt;
    let outcome = session
        .finish_compaction(
            attempt,
            &plan,
            Ok("  User pasted blobs; keep going.\n".into()),
        )
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Compacted {
            turns: 5,
            send: true
        }
    );
    assert!(!session.compacting());
    assert_eq!(session.messages, newest);
    assert_eq!(session.draft, "the new prompt", "the prompt is untouched");
    let note = session.compaction().unwrap();
    assert_eq!(note.turns, 5);
    assert_eq!(note.summary, "User pasted blobs; keep going.");
    assert_eq!(note.marker(), "5 earlier turns compacted into a summary");

    // The next request: note, kept turns verbatim, then the new prompt last.
    let request = session.begin().unwrap();
    assert_eq!(request[0].role, "system");
    assert!(request[0]
        .content
        .contains("User pasted blobs; keep going."));
    assert_eq!(&request[1..5], &newest[..]);
    assert_eq!(request.last().unwrap().content, "the new prompt");
    assert_eq!(request.len(), 1 + 4 + 1);
}

#[test]
fn repeated_compaction_merges_the_previous_note_and_sums_turns() {
    let mut session = long(7);
    compact(&mut session, "first note");
    for number in 8..=11 {
        turn(&mut session, number);
    }
    session.insert("go on");
    let plan = session.start_compaction(None, true).unwrap();
    assert_eq!(plan.transcript[0].role, "summary");
    assert_eq!(plan.transcript[0].content, "first note");
    let attempt = session.attempt;
    session
        .finish_compaction(attempt, &plan, Ok("merged".into()))
        .unwrap();
    assert_eq!(session.compaction().unwrap().turns, 5 + plan.turns);
    assert_eq!(session.compaction().unwrap().summary, "merged");
}

#[test]
fn index_bound_state_follows_the_kept_turns() {
    let mut session = long(7);
    let source_before = session.source(13).cloned();
    assert!(source_before.is_some());
    compact(&mut session, "note");
    assert_eq!(session.messages.len(), 4);
    assert_eq!(session.source(3).cloned(), source_before);
    assert!(session.source(13).is_none());
}

/// A turn whose reply was cancelled; it becomes an interrupted turn once the next one starts.
/// Issue #391: `@path` refusal rows follow their prompt through a compaction.
#[test]
fn mention_refusal_rows_follow_their_prompt_or_go_with_a_compacted_one() {
    let mut kept = long(7);
    kept.show_mention_refusals(12, vec!["@a.rs: not attached (binary)".into()]);
    compact(&mut kept, "note");
    let (after, rows) = kept.mention_refusals();
    assert_eq!((after, rows.len()), (2, 1));
    assert!(kept.messages[after].content.starts_with("turn7 "));

    let mut gone = long(7);
    gone.show_mention_refusals(2, vec!["@a.rs: not attached (binary)".into()]);
    compact(&mut gone, "note");
    assert!(gone.mention_refusals().1.is_empty());
}

fn cancelled_turn(session: &mut Session, number: usize) {
    session.insert(&format!("turn{number} {}", "x".repeat(PASTE - 10)));
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Token(format!("partial{number}")));
    session.cancel();
}

fn run_command(session: &mut Session, task: u64) {
    session
        .submit_command(
            format!("/run {task}"),
            alfredo_tui::command_intent::Intent::Run {
                correlation: format!("c{task}"),
                expected_revision: 1,
                task,
            },
        )
        .unwrap();
}

fn receipt(session: &mut Session, revision: u64) {
    let after_messages = session.messages.len();
    assert!(
        session.observe_task_receipt(alfredo_tui::model::TaskReceiptRef {
            sequence: 0,
            after_messages,
            revision,
            task: 1,
            correlation: format!("r{revision}"),
        })
    );
}

#[test]
fn message_numbered_state_is_renumbered_or_clamped_around_the_cut() {
    let mut session = Session::new("fixture".into());
    turn(&mut session, 1); // messages 0..2
    session.attach(0, "ATTACH-OLD".into());
    receipt(&mut session, 1); // after 2: compacted
    run_command(&mut session, 1); // after 2: compacted
    cancelled_turn(&mut session, 2); // messages 2..4, interrupted, compacted
    for number in 3..=5 {
        turn(&mut session, number); // messages 4..10
    }
    receipt(&mut session, 2); // after 10: the cut itself
    run_command(&mut session, 2);
    cancelled_turn(&mut session, 6); // messages 10..12, interrupted, kept
    turn(&mut session, 7); // messages 12..14
    session.attach(12, "ATTACH-KEPT".into());
    receipt(&mut session, 3); // after 14: kept
    run_command(&mut session, 3);
    assert_eq!(session.messages.len(), 14);
    assert!(session.interrupted_turn(2) && session.interrupted_turn(10));

    session.insert("next");
    let plan = session.start_compaction(None, true).unwrap();
    assert_eq!(plan.turns, 5);
    let sent: String = plan.transcript.iter().map(|m| m.content.as_str()).collect();
    assert!(
        !sent.contains("turn2 ") && !sent.contains("partial2"),
        "interrupted turn skipped"
    );
    assert!(sent.contains("turn1 ") && sent.contains("turn3 ") && sent.contains("turn5 "));
    let attempt = session.attempt;
    session
        .finish_compaction(attempt, &plan, Ok("note".into()))
        .unwrap();

    assert_eq!(session.messages.len(), 4);
    assert!(session.messages[0].content.starts_with("turn6 "));
    let interrupted: Vec<usize> = (0..4).filter(|i| session.interrupted_turn(*i)).collect();
    assert_eq!(
        interrupted,
        [0],
        "only the kept interrupted turn remains, renumbered"
    );
    // Attachments: the compacted one is gone, the kept one follows its message.
    let request = session.request_through(2);
    let all: String = request.iter().map(|m| m.content.as_str()).collect();
    assert!(!all.contains("ATTACH-OLD"));
    assert!(all.contains("ATTACH-KEPT"));
    // Placements inside the compacted range clamp to the top; later ones shift by the cut.
    let receipts: Vec<usize> = session
        .task_receipts()
        .iter()
        .map(|r| r.after_messages)
        .collect();
    assert_eq!(receipts, [0, 0, 4]);
    let commands: Vec<usize> = session
        .commands()
        .iter()
        .map(|c| c.after_messages)
        .collect();
    assert_eq!(commands, [0, 0, 4]);
    // The renumbered session is still a valid saved conversation and restores identically.
    use alfredo_tui::{
        conversations::{ConversationStore, Snapshot},
        tasks::TaskStore,
    };
    let root = std::env::temp_dir().join(format!(
        "alfredo-compaction-renumber-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let tasks = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let store = ConversationStore::open(&tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0] = session;
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let loaded = store.load().unwrap().unwrap().restore();
    let restored = &loaded.sessions[0];
    assert_eq!(restored.messages, app.sessions[0].messages);
    assert!(restored.interrupted_turn(0));
    assert_eq!(restored.task_receipts().len(), 3);
    assert_eq!(restored.commands().len(), 3);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn failure_keeps_history_and_prompt_and_names_the_ways_forward() {
    for result in [
        Err("Cannot reach Ollama".to_string()),
        Ok("   \n ".to_string()),
    ] {
        let mut session = long(7);
        let before = session.messages.clone();
        session.insert("keep me");
        let plan = session.start_compaction(None, true).unwrap();
        let attempt = session.attempt;
        let outcome = session.finish_compaction(attempt, &plan, result).unwrap();
        let Outcome::Failed(_) = &outcome else {
            panic!("expected failure, got {outcome:?}");
        };
        let notice = outcome.notice();
        assert!(
            notice.contains("Ctrl+N") && notice.contains("/clear"),
            "{notice}"
        );
        assert!(notice.contains("draft"), "{notice}");
        assert!(
            notice.find("/clear").unwrap() < 90,
            "ways forward stay visible: {notice}"
        );
        assert_eq!(session.messages, before);
        assert_eq!(session.draft, "keep me");
        assert!(session.compaction().is_none());
        assert!(!session.compacting());
    }
}

#[test]
fn late_or_cancelled_results_change_nothing() {
    let mut session = long(7);
    session.insert("p");
    let plan = session.start_compaction(None, true).unwrap();
    assert!(session.cancel_compaction());
    let attempt = session.attempt;
    assert!(session
        .finish_compaction(attempt, &plan, Ok("late".into()))
        .is_none());
    assert_eq!(session.messages.len(), 14);
    // A result for another attempt is ignored too.
    let plan = session.start_compaction(None, true).unwrap();
    assert!(session
        .finish_compaction(attempt + 9, &plan, Ok("stale".into()))
        .is_none());
    assert!(session.compaction().is_none());
}

#[test]
fn a_prompt_edited_while_summarising_is_not_sent_automatically() {
    let mut session = long(7);
    session.insert("first draft");
    let plan = session.start_compaction(None, true).unwrap();
    session.insert(" and more");
    let attempt = session.attempt;
    let outcome = session
        .finish_compaction(attempt, &plan, Ok("note".into()))
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Compacted {
            turns: 5,
            send: false
        }
    );
}

#[test]
fn manual_compaction_needs_earlier_turns_and_does_not_send() {
    let mut session = long(1);
    assert!(session
        .start_compaction(None, false)
        .unwrap_err()
        .contains("Nothing to compact"));
    let mut session = long(3);
    let plan = session.start_compaction(None, false).unwrap();
    assert_eq!(plan.turns, 1);
    let attempt = session.attempt;
    let outcome = session
        .finish_compaction(attempt, &plan, Ok("n".into()))
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Compacted {
            turns: 1,
            send: false
        }
    );
    assert_eq!(session.messages.len(), 4);
}

#[test]
fn the_remaining_hard_cap_names_ctrl_n_and_clear() {
    let mut big = Session::new("m".into());
    for _ in 0..8 {
        big.insert(&"q".repeat(alfredo_tui::model::MAX_DRAFT));
        if big.begin().is_err() {
            break;
        }
        let attempt = big.attempt;
        big.apply(attempt, Update::Done);
    }
    big.insert(&"q".repeat(alfredo_tui::model::MAX_DRAFT));
    let error = big.begin().unwrap_err();
    assert_eq!(error, HARD_CAP);
    assert!(error.contains("Ctrl+N") && error.contains("/clear"));
}

#[test]
fn clear_starts_this_conversation_fresh_and_keeps_the_model() {
    let mut session = long(7);
    compact(&mut session, "note");
    session.model = "other-model".into();
    let attempt = session.attempt;
    session.clear_conversation(None).unwrap();
    assert!(session.messages.is_empty() && session.compaction().is_none());
    assert_eq!(session.model, "other-model");
    assert_eq!(session.status, Status::Ready);
    assert_eq!(
        session.attempt, attempt,
        "late events from the old reply stay stale"
    );
    session.insert("fresh");
    assert_eq!(session.begin().unwrap().len(), 1);
}

#[test]
fn clear_refuses_while_a_reply_or_summary_is_running() {
    let mut session = long(1);
    session.insert("go");
    session.begin().unwrap();
    assert!(session.clear_conversation(None).is_err());
    let mut session = long(7);
    session.insert("go");
    session.start_compaction(None, true).unwrap();
    assert!(session.clear_conversation(None).is_err());
}

#[test]
fn the_note_survives_save_and_restore_and_bad_notes_are_refused() {
    let mut session = long(7);
    compact(&mut session, "persisted note");
    let json = serde_json::to_value(&session).unwrap();
    assert_eq!(json["compaction"]["turns"], 5);
    let restored: Session = serde_json::from_value(json).unwrap();
    assert_eq!(restored.compaction().unwrap().summary, "persisted note");
    assert_eq!(restored.messages, session.messages);
    let plain = serde_json::to_value(long(1)).unwrap();
    assert!(
        plain.get("compaction").is_none(),
        "unchanged files stay unchanged"
    );
}

#[test]
fn the_transcript_shows_the_marker_and_the_note() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut app = App::new("fixture".into());
    for number in 1..=4 {
        let session = &mut app.sessions[0];
        session.insert(&format!("small question {number}"));
        session.begin().unwrap();
        session.apply(number as u64, Update::Token("short answer".into()));
        session.apply(number as u64, Update::Done);
    }
    let plan = app.sessions[0].start_compaction(None, false).unwrap();
    app.sessions[0]
        .finish_compaction(4, &plan, Ok("The user pasted blobs.".into()))
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(
        screen.contains("2 earlier turns compacted into a summary"),
        "{screen}"
    );
    assert!(screen.contains("The user pasted blobs."), "{screen}");
}

#[test]
fn summary_request_is_bounded_and_fits_a_small_window() {
    let mut session = long(7);
    session.insert("p");
    let plan = session.start_compaction(None, true).unwrap();
    let request = compaction::request(&plan, None);
    assert_eq!(request.len(), 2);
    assert!(
        request[1].content.len() <= 65_000,
        "{}",
        request[1].content.len()
    );
    for window in [2048, 4096, 8192] {
        let small = compaction::request(&plan, Some(window));
        // The provider refuses prompts whose estimate (bytes/3 + reply budget) passes the window.
        let estimate: usize = small
            .iter()
            .map(|m| m.content.len() + m.role.len())
            .sum::<usize>()
            / 3
            + compaction::tokens(Some(window)) as usize;
        assert!(estimate <= window as usize, "{window}: {estimate}");
    }
}

// #244: the cut leaves room for the omitted-bytes suffix, so the note never exceeds
// SUMMARY_MAX, at every length around the boundary and on char boundaries.
#[test]
fn clean_summary_never_exceeds_the_limit_for_any_length_near_the_boundary() {
    use alfredo_tui::model::compaction::{clean_summary, SUMMARY_MAX};
    for len in (SUMMARY_MAX - 80)..(SUMMARY_MAX + 80) {
        for unit in ["a", "é", "日", "😀"] {
            let text = unit.repeat(len);
            let cleaned = clean_summary(&text).unwrap();
            assert!(
                cleaned.len() <= SUMMARY_MAX,
                "{unit} x {len}: {} bytes",
                cleaned.len()
            );
            assert!(std::str::from_utf8(cleaned.as_bytes()).is_ok());
        }
    }
    let short = "a".repeat(SUMMARY_MAX);
    assert_eq!(clean_summary(&short).unwrap(), short, "fits unchanged");
}

#[test]
fn a_compacted_conversation_saves_and_loads_and_a_corrupt_note_is_refused() {
    use alfredo_tui::{
        conversations::{ConversationStore, Snapshot},
        tasks::TaskStore,
    };
    let root = std::env::temp_dir().join(format!("alfredo-compaction-{}", std::process::id()));
    std::fs::create_dir_all(root.join("workspace")).unwrap();
    let tasks = TaskStore::new(&root.join("state"), &root.join("workspace"), "mission").unwrap();
    let store = ConversationStore::open(&tasks, "default").unwrap();
    let mut app = App::new("fixture".into());
    app.sessions[0] = long(7);
    compact(&mut app.sessions[0], "kept across restarts");
    store.save(&Snapshot::capture(&app, "default")).unwrap();
    let loaded = store.load().unwrap().unwrap().restore();
    let session = &loaded.sessions[0];
    assert_eq!(
        session.compaction().unwrap().summary,
        "kept across restarts"
    );
    assert_eq!(session.messages.len(), 4);
    // An empty note is not a valid saved conversation.
    let mut value = serde_json::to_value(Snapshot::capture(&app, "default")).unwrap();
    value["sessions"][0]["compaction"]["summary"] = " ".into();
    let bad: Snapshot = serde_json::from_value(value).unwrap();
    assert!(store.save(&bad).is_err());
    let _ = std::fs::remove_dir_all(root);
}
