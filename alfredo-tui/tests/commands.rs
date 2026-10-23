use alfredo_tui::{
    commands::Completion,
    conversations::Snapshot,
    model::{App, Session},
    ui,
};
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn history_traversal_restores_unsent_unicode_draft_and_cursor() {
    let mut session = Session::new("fixture".into());
    for text in ["first", "/models", "third"] {
        session.insert(text);
        session.remember_submission();
        session.clear_draft();
    }
    session.insert("A界Z");
    session.left();
    session.history_previous();
    assert_eq!(session.draft, "third");
    session.history_previous();
    assert_eq!(session.draft, "/models");
    session.history_next();
    session.history_next();
    session.insert("🦀");
    assert_eq!(session.draft, "A界🦀Z");
}

#[test]
fn completion_filters_cycles_and_only_produces_a_draft() {
    let mut picker = Completion::open("/mod").unwrap();
    assert_eq!(picker.draft(), "/models ");
    picker.next(true);
    assert_eq!(picker.draft(), "/model ");
    picker.next(false);
    assert_eq!(picker.draft(), "/models ");
    assert!(Completion::open("/run 1").is_none());
    assert!(Completion::open("question").is_none());
    assert!(Completion::open("/unknown").is_none());
    let mut app = App::new("fixture".into());
    app.completion = Some(picker);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Enter fills draft"));
    assert!(text.contains("/models"));
    assert!(app.sessions[0].messages.is_empty());
}

#[test]
fn slash_typeahead_lists_prefix_then_substring_matches_with_usage() {
    let all = Completion::typeahead("/").unwrap();
    assert!(all.live && all.choices.len() > 30);
    let picker = Completion::typeahead("/pla").unwrap();
    let names: Vec<_> = picker.choices.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names[0], "/plan");
    assert!(names.contains(&"/plan-save"), "{names:?}");
    // `/resume` matches `/re` by prefix; `/plan-revise` contains it only inside.
    let names: Vec<_> = Completion::typeahead("/re")
        .unwrap()
        .choices
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert!(
        names.iter().position(|n| n == "/resume").unwrap()
            < names.iter().position(|n| n == "/plan-revise").unwrap()
    );
    assert!(Completion::typeahead("/go").unwrap().choices[0]
        .description
        .contains("GOAL"));
    assert_eq!(Completion::typeahead("/pla").unwrap().draft(), "/plan ");
    // An exact name leads, so Enter on a fully typed `/model` still submits it.
    let exact = Completion::typeahead("/model").unwrap();
    assert!(exact.selected_is("/model") && exact.choices.len() > 1);
    for draft in ["", "hello", "@way", "/path/in chat", "/zzzz-none"] {
        assert!(Completion::typeahead(draft).is_none(), "{draft}");
    }
}

#[test]
fn f1_picker_filters_by_typed_text() {
    let mut help = Completion::all();
    let total = help.choices.len();
    help.set_query("pla".into());
    assert!(help.choices.len() < total && help.choices[0].name == "/plan");
    assert!(help.choices.iter().all(|c| c.group == Some("Matches")));
    assert_eq!(help.draft(), "/plan ");
    help.set_query("zzzz".into());
    assert!(help.choices.is_empty());
    help.next(true);
    help.set_query(String::new());
    assert_eq!(help.choices.len(), total);
    assert!(help.help && !help.live);
}

#[test]
fn live_typeahead_renders_matches_and_usage_hints() {
    let mut app = App::new("fixture".into());
    app.completion = Completion::typeahead("/pla");
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("/plan-save"), "{text}");
    assert!(text.contains("Enter/Tab fills draft"), "{text}");
    app.completion.as_mut().unwrap().set_query("x".into());
}

#[test]
fn autosave_during_history_browsing_preserves_the_original_unsent_draft() {
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("submitted");
    app.sessions[0].begin().unwrap();
    app.sessions[0].cancel();
    app.sessions[0].insert("unsent");
    app.sessions[0].left();
    app.sessions[0].history_previous();
    assert_eq!(app.sessions[0].draft, "submitted");
    let snapshot = Snapshot::capture(&app, "default");
    let serialized = serde_json::to_value(&snapshot).unwrap();
    assert!(serialized["sessions"][0].get("history").is_none());
    let mut restored = snapshot.restore();
    restored.sessions[0].insert("X");
    assert_eq!(restored.sessions[0].draft, "unsenXt");
    restored.sessions[0].history_previous();
    assert_eq!(restored.sessions[0].draft, "submitted");
}

#[test]
fn history_is_bounded_and_does_not_cross_sessions() {
    let mut app = App::new("fixture".into());
    for index in 0..110 {
        app.sessions[0].insert(&format!("command-{index}"));
        app.sessions[0].remember_submission();
        app.sessions[0].clear_draft();
    }
    app.add_session();
    app.sessions[1].insert("second draft");
    app.sessions[1].history_previous();
    assert_eq!(app.sessions[1].draft, "second draft");
    for _ in 0..200 {
        app.sessions[0].history_previous();
    }
    assert_eq!(app.sessions[0].draft, "command-10");
}

#[test]
fn installed_model_completion_preserves_assignment_and_has_no_effect() {
    let models = vec![
        "zeta:14b".into(),
        "界:8b".into(),
        "alpha:7b".into(),
        "alpha:7b".into(),
        "bad\nname".into(),
    ];
    let mut picker = Completion::with_models("/model ", &models).unwrap();
    assert_eq!(picker.choices.len(), 3);
    assert_eq!(picker.draft(), "/model alpha:7b");
    picker.next(false);
    assert_eq!(picker.draft(), "/model 界:8b");
    let picker = Completion::with_models("/assign 42 界", &models).unwrap();
    assert_eq!(picker.draft(), "/assign 42 界:8b");
    let mut app = App::new("original".into());
    app.sessions[0].insert("/assign 42 界");
    let before = Snapshot::capture(&app, "default");
    app.completion = Some(picker);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    assert_eq!(Snapshot::capture(&app, "default"), before);
    assert_eq!(app.sessions[0].model, "original");
    for draft in [
        "/assign 0 a",
        "/assign x a",
        "/assign 42 a suffix",
        "/model a suffix",
        "plain text",
    ] {
        assert!(!Completion::accepts(draft));
        assert!(Completion::with_models(draft, &models).is_none());
    }
    assert!(Completion::accepts("/model missing"));
    assert!(Completion::with_models("/model missing", &models).is_none());
    assert!(Completion::with_models("/model ", &[]).is_none());
}

#[test]
fn capability_completion_and_parsing_only_admit_native_routes() {
    use alfredo_tui::commands::capability_prompt;
    let picker = Completion::open("@way").unwrap();
    assert_eq!(picker.draft(), "@wayfinder ");
    assert!(Completion::accepts("@way"));
    assert!(Completion::open("@unknown").is_none());
    assert!(Completion::open("@wayfinder request").is_none());
    assert!(Completion::all()
        .choices
        .iter()
        .any(|choice| choice.name == "@wayfinder"));
    for prompt in ["@unknown run", "@wayfinder", "@wayfinderish request"] {
        assert!(capability_prompt(prompt).is_err());
    }
    assert_eq!(
        capability_prompt("  @wayfinder inspect scope").unwrap(),
        Some("inspect scope")
    );
    assert_eq!(capability_prompt("mention @wayfinder later").unwrap(), None);
    assert_eq!(
        capability_prompt("@calc.py what does it return").unwrap(),
        None
    );
    assert_eq!(capability_prompt("@src/main.rs explain").unwrap(), None);
    let mut app = App::new("fixture".into());
    app.completion = Some(picker);
    for (width, height) in [(100, 24), (32, 10)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("@wayfinder"));
    }
    assert!(app.sessions[0].messages.is_empty());
}
