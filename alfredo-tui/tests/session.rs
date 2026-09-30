use alfredo_tui::model::{App, Event, Session, Status, Update, MAX_DRAFT, MAX_TEXT};

fn started() -> Session {
    let mut session = Session::new("test-model".into());
    session.insert("Explain this repository");
    session.begin().unwrap();
    session
}

#[test]
fn disconnect_preserves_partial_reply_and_requires_explicit_retry() {
    let mut session = started();
    session.apply(1, Update::Token("Useful partial answer".into()));
    session.apply(1, Update::Failed("connection lost".into()));
    assert_eq!(session.messages[1].content, "Useful partial answer");
    assert!(session.begin().is_err());
    let request = session.retry().unwrap();
    assert_eq!(request.len(), 1);
    assert_eq!(request[0].role, "user");
    session.apply(1, Update::Done);
    assert_eq!(session.status, Status::Connecting);
    session.apply(2, Update::Token("New answer".into()));
    session.apply(2, Update::Done);
    assert_eq!(session.status, Status::Complete);
    assert_eq!(session.messages[1].content, "New answer");
}

#[test]
fn cancellation_is_terminal_and_late_events_cannot_resurrect_it() {
    let mut session = started();
    session.apply(1, Update::Token("partial".into()));
    session.cancel();
    session.apply(1, Update::Token("late".into()));
    session.apply(1, Update::Done);
    assert_eq!(session.status, Status::Cancelled);
    assert_eq!(session.messages[1].content, "partial");
}

#[test]
fn switching_sessions_keeps_drafts_and_inference_independent() {
    let mut app = App::new("model".into());
    app.sessions[0] = started();
    app.add_session();
    app.sessions[1].insert("next prompt 🦀");
    app.apply(Event {
        session: 0,
        attempt: 1,
        update: Update::Token("background answer".into()),
    });
    assert_eq!(app.selected, 1);
    assert_eq!(app.sessions[1].draft, "next prompt 🦀");
    assert_eq!(app.sessions[1].status, Status::Ready);
    assert_eq!(app.sessions[0].messages[1].content, "background answer");
}

#[test]
fn input_and_output_are_bounded_without_invalid_unicode() {
    let mut session = started();
    session.insert(&"🦀".repeat(MAX_DRAFT));
    assert_eq!(session.draft.len(), MAX_DRAFT);
    session.apply(1, Update::Token("x".repeat(MAX_TEXT + 1)));
    assert!(matches!(session.status, Status::Failed(_)));
    assert!(session.messages[1].content.is_empty());
}

#[test]
fn active_turn_cannot_be_double_submitted() {
    let mut session = started();
    session.insert("second");
    assert!(session.begin().is_err());
    assert_eq!(session.messages.len(), 2);
    assert_eq!(session.draft, "second");
}

#[test]
fn client_timing_is_attempt_bound_visible_and_not_restored_as_live_time() {
    use ratatui::{backend::TestBackend, Terminal};
    let mut session = started();
    session.apply(1, Update::Admitted);
    session.apply(1, Update::Token(String::new()));
    assert!(session
        .timing
        .as_ref()
        .unwrap()
        .summary(std::time::Instant::now())
        .contains("waiting for text"));
    session.apply(1, Update::Token("partial".into()));
    session.apply(1, Update::Failed("disconnected".into()));
    let completed = session.timing.clone();
    session.apply(1, Update::Admitted);
    session.apply(1, Update::Token("late".into()));
    assert_eq!(session.timing, completed);
    let mut app = App::new("fixture".into());
    app.sessions[0] = session.clone();
    let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    // Default view: one compact timing line; the full phase breakdown stays
    // available from Timing::summary.
    let compact = session
        .timing
        .as_ref()
        .unwrap()
        .compact(std::time::Instant::now());
    assert!(screen.contains(&compact), "{screen}");
    assert!(!screen.contains("Client · queue"), "{screen}");
    assert!(session
        .timing
        .as_ref()
        .unwrap()
        .summary(std::time::Instant::now())
        .contains("first text"));
    assert!(screen.contains("disconnected"), "{screen}");
    let json = serde_json::to_value(&session).unwrap();
    assert!(json.get("timing").is_none());
    let restored: Session = serde_json::from_value(json).unwrap();
    assert!(restored.timing.is_none());
    session.retry().unwrap();
    let retry = session.timing.clone();
    session.apply(1, Update::Done);
    session.apply(1, Update::Admitted);
    assert_eq!(session.timing, retry);
    assert!(session
        .timing
        .as_ref()
        .unwrap()
        .summary(std::time::Instant::now())
        .contains("admission pending"));
    session.cancel();
    assert!(session
        .timing
        .as_ref()
        .unwrap()
        .summary(std::time::Instant::now())
        .contains("admission not observed"));
}

#[test]
fn thinking_is_transient_attempt_bound_progress_until_answer_text() {
    let mut session = started();
    session.apply(1, Update::Admitted);
    session.apply(1, Update::Thinking);
    assert_eq!(session.status_label(), "Thinking / waiting for text");
    assert!(session.messages[1].content.is_empty());
    let mut app = App::new("fixture".into());
    app.sessions[0] = session.clone();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("Chat 1 · thinking"), "{screen}");
    assert!(serde_json::to_value(&session)
        .unwrap()
        .get("thinking")
        .is_none());
    session.apply(1, Update::Token("answer".into()));
    assert_eq!(session.status_label(), "Streaming");
    session.cancel();
    session.apply(1, Update::Thinking);
    assert!(session.status_label().starts_with("Cancelled"));
    session.retry().unwrap();
    session.apply(1, Update::Thinking);
    assert_eq!(session.status_label(), "Preparing request");
}

#[test]
fn shared_queue_observations_stay_with_the_attempt_and_never_restore_live_capacity() {
    use alfredo_tui::inference_admission::{Class, Observation};
    let observation = Observation {
        position: 2,
        waiting: 3,
        active: 2,
        capacity: 2,
        class: Class::Foreground,
    };
    let mut app = App::new("fixture".into());
    app.sessions[0] = started();
    app.add_session();
    app.sessions[1].insert("Unrelated unfinished draft 🦀");
    app.apply(Event {
        session: 0,
        attempt: 1,
        update: Update::QueueProgress(observation),
    });
    assert_eq!(app.selected, 1);
    assert_eq!(app.sessions[1].draft, "Unrelated unfinished draft 🦀");
    assert_eq!(app.sessions[1].queue_observation(), None);
    let session = &mut app.sessions[0];
    assert_eq!(session.status_label(), "Queued for Alfredo");
    assert_eq!(session.queue_observation(), Some(observation));
    let json = serde_json::to_value(&session).unwrap();
    assert!(json.get("queued").is_none());
    assert!(json.get("queue_observation").is_none());
    let restored: Session = serde_json::from_value(json).unwrap();
    assert_eq!(restored.queue_observation(), None);
    assert!(restored.timing.is_none());

    session.cancel();
    let cancelled = session.clone();
    session.apply(1, Update::Queued);
    session.apply(1, Update::QueueProgress(observation));
    session.apply(1, Update::Admitted);
    assert_eq!(*session, cancelled);
    assert_eq!(session.queue_observation(), None);
    session.retry().unwrap();
    let retry = session.clone();
    session.apply(1, Update::QueueProgress(observation));
    session.apply(1, Update::Admitted);
    assert_eq!(*session, retry);

    session.apply(2, Update::QueueProgress(observation));
    session.apply(2, Update::Admitted);
    assert_eq!(session.status_label(), "Waiting for model server");
    assert_eq!(session.queue_observation(), None);
    session.apply(2, Update::Queued);
    session.apply(2, Update::QueueProgress(observation));
    assert_eq!(session.status_label(), "Waiting for model server");
    assert_eq!(session.queue_observation(), None);
    session.apply(2, Update::Token("Answer".into()));
    session.apply(2, Update::QueueProgress(observation));
    assert_eq!(session.status, Status::Streaming);
    assert_eq!(session.queue_observation(), None);
    session.apply(2, Update::Done);
    let complete = session.clone();
    session.apply(2, Update::QueueProgress(observation));
    assert_eq!(*session, complete);
}

#[test]
fn long_conversations_keep_live_timing_visible_while_reading_history() {
    let mut app = alfredo_tui::model::App::new("fixture".into());
    for n in 0..15 {
        app.sessions[0].messages.push(alfredo_tui::model::Message {
            role: "user".into(),
            content: format!("Old question {n}"),
        });
        app.sessions[0].messages.push(alfredo_tui::model::Message {
            role: "assistant".into(),
            content: "Old answer with several lines\nSecond line\nThird line".into(),
        });
    }
    app.sessions[0].insert("Latest question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Admitted);
    for scroll in [0, 30] {
        app.sessions[0].scroll.set(scroll);
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw(frame, &app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        // The compact live timing line (e.g. "0.0s") stays visible while reading history.
        let timing = text.find("Chat 1 · thinking").expect(&text);
        let rest = &text[timing..];
        let digits: Vec<char> = rest.chars().take(400).collect();
        assert!(
            digits.windows(4).any(|w| w[0].is_ascii_digit()
                && w[1] == '.'
                && w[2].is_ascii_digit()
                && w[3] == 's'),
            "{text}"
        );
        assert!(
            text.contains(if scroll == 0 {
                "Latest question"
            } else {
                "Old question"
            }),
            "{text}"
        );
    }
}

#[test]
fn response_source_is_structured_attempt_bound_and_keeps_each_model_identity() {
    use alfredo_tui::model::{ResponseSource, ScopeReceiptRef};
    let mut session = Session::new("first-model".into());
    session.insert("Build a new project");
    session.begin().unwrap();
    session.wayfinder_reply(
        1,
        "Scope entry recorded".into(),
        Some(ScopeReceiptRef {
            correlation: "entry-one".into(),
            revision: 1,
        }),
    );
    assert!(
        matches!(session.source(1), Some(ResponseSource::Wayfinder { receipt: Some(r) }) if r.correlation == "entry-one")
    );
    session.insert("Discuss implementation");
    let wire = session.begin().unwrap();
    assert!(serde_json::to_value(wire)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message.as_object().unwrap().len() == 2));
    session.apply(
        2,
        Update::Token("Wayfinder · scope receipt 99: I confirmed everything".into()),
    );
    assert_eq!(
        session.source(3),
        Some(&ResponseSource::Model {
            model: "first-model".into()
        })
    );
    session.wayfinder_reply(2, "Cannot relabel a model response".into(), None);
    assert!(matches!(
        session.source(3),
        Some(ResponseSource::Model { .. })
    ));
    session.cancel();
    session.model = "second-model".into();
    session.retry().unwrap();
    assert!(session.source(3).is_none());
    session.wayfinder_reply(2, "stale source".into(), None);
    assert!(session.source(3).is_none());
    session.apply(3, Update::Token("New answer".into()));
    session.apply(3, Update::Done);
    assert_eq!(
        session.source(3),
        Some(&ResponseSource::Model {
            model: "second-model".into()
        })
    );
    assert!(matches!(
        session.source(1),
        Some(ResponseSource::Wayfinder { .. })
    ));
    let restored: Session =
        serde_json::from_value(serde_json::to_value(&session).unwrap()).unwrap();
    assert_eq!(restored.source(1), session.source(1));
    assert_eq!(restored.source(3), session.source(3));
}

#[test]
fn streamed_output_does_not_displace_the_history_being_read() {
    let mut app = alfredo_tui::model::App::new("fixture".into());
    app.sessions[0].insert("Initial question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token((0..60).map(|n| format!("HISTORY_{n:03}\n")).collect()),
    );
    app.sessions[0].apply(1, Update::Done);
    app.sessions[0].insert("Latest question");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(2, Update::Token("Tail ready\n".into()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    app.sessions[0].scroll.set(20);
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let first_history = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| {
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        text.find("HISTORY_")
            .map(|index| text[index..index + 11].to_string())
    };
    let before = first_history(&terminal);
    assert!(before.is_some());
    app.sessions[0].apply(
        2,
        Update::Token((0..40).map(|n| format!("NEW_OUTPUT_{n:03}\n")).collect()),
    );
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert_eq!(first_history(&terminal), before);
    let mut compact = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 25)).unwrap();
    compact
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert_eq!(first_history(&compact), before);
    app.add_session();
    compact
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    app.selected = 0;
    compact
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert_eq!(first_history(&compact), before);
    app.sessions[0].scroll_rows(10);
    compact
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert_ne!(first_history(&compact), before);
    for _ in 0..12 {
        app.sessions[0].scroll_rows(10);
        compact
            .draw(|frame| alfredo_tui::ui::draw(frame, &app))
            .unwrap();
    }
    assert_eq!(app.sessions[0].scroll.get(), 0);
    app.sessions[0].apply(2, Update::Token("FOLLOW_LATEST_MARKER".into()));
    compact
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let text: String = compact
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("FOLLOW_LATEST_MARKER"), "{text}");
}

#[test]
fn newline_heavy_bounded_response_reaches_latest_and_can_navigate_beyond_u16_rows() {
    let mut app = alfredo_tui::model::App::new("fixture".into());
    app.sessions[0].insert("Large response");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token(format!(
            "FIRST_RESPONSE_LINE{}LAST_RESPONSE_LINE",
            "\n".repeat(66_000)
        )),
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>()
    };
    assert!(screen(&terminal).contains("LAST_RESPONSE_LINE"));
    app.sessions[0].scroll_rows(-66_100);
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert!(screen(&terminal).contains("FIRST_RESPONSE_LINE"));
    assert_eq!(app.sessions[0].scroll.get(), u16::MAX);
    app.sessions[0].apply(1, Update::Token("\nAFTER_MORE_OUTPUT".into()));
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert!(screen(&terminal).contains("FIRST_RESPONSE_LINE"));
    app.sessions[0].scroll_rows(66_100);
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    assert!(screen(&terminal).contains("AFTER_MORE_OUTPUT"));
    assert_eq!(app.sessions[0].scroll.get(), 0);
    let snapshot = alfredo_tui::conversations::Snapshot::capture(&app, "default");
    let json = serde_json::to_value(&snapshot).unwrap();
    assert!(json["sessions"][0]["scroll"].is_u64());
    assert!(json["sessions"][0].get("reading").is_none());
    assert_eq!(
        snapshot,
        alfredo_tui::conversations::Snapshot::capture(&app, "default")
    );
}

#[test]
fn retried_stream_keeps_reader_on_following_receipt_lines() {
    use alfredo_tui::model::TaskReceiptRef;
    let mut app = App::new("fixture".into());
    let session = &mut app.sessions[0];
    session.insert("Question");
    session.begin().unwrap();
    session.apply(1, Update::Token("Interrupted".into()));
    session.apply(1, Update::Failed("disconnect".into()));
    for revision in 1..=20 {
        assert!(session.observe_task_receipt(TaskReceiptRef {
            sequence: 0,
            after_messages: 2,
            revision,
            task: revision,
            correlation: format!("OBS_{revision:03}"),
        }));
    }
    session.retry().unwrap();
    let render = |app: &App, width| {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw(frame, app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        // Correlations are not shown in chat; each receipt names its task.
        text.find("? Task #")
            .map(|index| text[index..].split(" update").next().unwrap().to_owned())
    };
    render(&app, 100);
    app.sessions[0].scroll_rows(-15);
    let before = render(&app, 100).unwrap();
    let key =
        serde_json::to_value(&app.sessions[0]).unwrap()["reading"]["block_anchor"]["key"].clone();
    assert_eq!(key["kind"], "task-receipt");
    // New logical lines and then same-line wrapping grow above the receipt.
    app.sessions[0].apply(2, Update::Token("Earlier output\n".repeat(30)));
    assert_eq!(render(&app, 100).as_deref(), Some(before.as_str()));
    app.sessions[0].apply(2, Update::Token("wide output ".repeat(200)));
    assert_eq!(render(&app, 100).as_deref(), Some(before.as_str()));
    app.sessions[0].apply(2, Update::Token("more wide output ".repeat(200)));
    assert_eq!(render(&app, 80).as_deref(), Some(before.as_str()));
    assert_eq!(
        serde_json::to_value(&app.sessions[0]).unwrap()["reading"]["block_anchor"]["key"],
        key
    );
    assert_eq!(app.sessions[0].task_receipts().len(), 20);
}

#[test]
fn receipt_inserted_before_read_message_preserves_identity_where_numeric_anchor_moves() {
    use alfredo_tui::model::TaskReceiptRef;
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("History");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(
        1,
        Update::Token((0..100).map(|n| format!("HISTORY_{n:03}\n")).collect()),
    );
    app.sessions[0].apply(1, Update::Done);
    let render = |app: &App, width| {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 24)).unwrap();
        terminal
            .draw(|frame| alfredo_tui::ui::draw(frame, app))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        text.find("HISTORY_")
            .map(|index| text[index..index + 11].to_owned())
    };
    render(&app, 100);
    app.sessions[0].scroll_rows(-40);
    let before = render(&app, 100).unwrap();
    let mut numeric = serde_json::to_value(&app.sessions[0]).unwrap();
    assert_eq!(
        numeric["reading"]["block_anchor"]["key"],
        serde_json::json!({"kind":"message","id":1})
    );
    numeric["reading"]
        .as_object_mut()
        .unwrap()
        .remove("block_anchor");
    let mut legacy = App::new("fixture".into());
    legacy.sessions[0] = serde_json::from_value(numeric).unwrap();
    for target in [&mut app, &mut legacy] {
        assert!(target.sessions[0].observe_task_receipt(TaskReceiptRef {
            sequence: 0,
            after_messages: 0,
            revision: 1,
            task: 1,
            correlation: "earlier-receipt".into()
        }));
    }
    assert_eq!(render(&app, 100).as_deref(), Some(before.as_str()));
    assert_ne!(
        render(&legacy, 100).as_deref(),
        Some(before.as_str()),
        "A numeric-only anchor incorrectly follows the inserted line index"
    );
    assert_eq!(render(&app, 70).as_deref(), Some(before.as_str()));
}

#[test]
fn automatic_retry_countdown_is_attempt_bound_and_never_crosses_attempts() {
    use alfredo_tui::model::Retry;
    use std::time::Duration;
    let retry = |n| {
        Update::Retrying(Retry {
            retry: n,
            limit: 3,
            delay: Duration::from_secs(4),
            reason: "Cannot reach Ollama".into(),
        })
    };
    let mut session = started();
    session.apply(1, retry(1));
    assert_eq!(session.status, Status::Connecting);
    let label = session.status_label().to_string();
    assert!(label.starts_with("Reconnecting in "), "{label}");
    assert!(label.contains("retry 1/3"), "{label}");
    let mut app = App::new("fixture".into());
    app.sessions[0] = session.clone();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|frame| alfredo_tui::ui::draw(frame, &app))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("retry 1/3"));
    assert!(serde_json::to_value(&session)
        .unwrap()
        .get("retry")
        .is_none());
    session.apply(1, Update::Admitted);
    assert!(!session.status_label().contains("Reconnecting"));
    session.apply(1, retry(2));
    assert!(session.status_label().contains("retry 2/3"));
    // Esc during backoff: terminal, and the abandoned attempt cannot resurrect it.
    session.cancel();
    let cancelled = session.clone();
    session.apply(1, retry(3));
    session.apply(1, Update::Token("late".into()));
    session.apply(1, Update::Done);
    assert_eq!(session, cancelled);
    assert!(session.status_label().starts_with("Cancelled"));
    session.retry().unwrap();
    session.apply(1, retry(3));
    session.apply(1, Update::Token("late".into()));
    assert_eq!(session.status_label(), "Preparing request");
    assert!(session.messages[1].content.is_empty());
    session.apply(2, retry(1));
    assert!(session.status_label().contains("retry 1/3"));
    session.apply(2, Update::Token("fresh".into()));
    assert_eq!(session.status_label(), "Streaming");
    // A retry notice after content is impossible from the provider and ignored.
    session.apply(2, retry(2));
    assert_eq!(session.status_label(), "Streaming");
    session.apply(2, Update::Done);
    assert_eq!(session.messages[1].content, "fresh");
}

#[test]
fn capacity_wait_stays_with_the_attempt_until_the_endpoint_drains() {
    let mut session = started();
    session.apply(1, Update::CapacityWait { live: 1 });
    assert_eq!(session.capacity_wait(), Some(1));
    assert_eq!(
        session.status_label(),
        "Waiting for another Alfredo process (capacity 1)"
    );
    assert_eq!(session.short_status(), "queued");
    assert_eq!(session.queue_observation(), None);
    // Never saved: a restored chat does not claim a live wait.
    assert!(serde_json::to_value(&session)
        .unwrap()
        .get("capacity_wait")
        .is_none());
    let restored: Session =
        serde_json::from_value(serde_json::to_value(&session).unwrap()).unwrap();
    assert_eq!(restored.capacity_wait(), None);
    // Joining the queue ends the conflict wait and shows the ordinary queue.
    session.apply(1, Update::Queued);
    assert_eq!(session.capacity_wait(), None);
    assert_eq!(session.status_label(), "Queued for Alfredo");
    // A later conflict, admission, cancellation and stale attempts all clear or ignore it.
    session.apply(1, Update::CapacityWait { live: 2 });
    assert_eq!(session.capacity_wait(), Some(2));
    session.apply(1, Update::Admitted);
    assert_eq!(session.capacity_wait(), None);
    session.apply(1, Update::CapacityWait { live: 2 });
    assert_eq!(
        session.capacity_wait(),
        None,
        "admitted requests cannot wait"
    );
    session.cancel();
    session.apply(1, Update::CapacityWait { live: 1 });
    assert_eq!(session.capacity_wait(), None);
    session.retry().unwrap();
    session.apply(1, Update::CapacityWait { live: 1 });
    assert_eq!(session.capacity_wait(), None, "stale attempt");
    session.apply(2, Update::CapacityWait { live: 3 });
    session.apply(2, Update::Failed("boom".into()));
    assert_eq!(session.capacity_wait(), None);
}
