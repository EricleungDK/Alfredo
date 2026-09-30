use alfredo_tui::{model::App, ui};
use ratatui::{backend::TestBackend, Terminal};

fn app() -> App {
    let mut app = App::new("b".into());
    app.models_visible = true;
    app.receive_models(Ok(vec!["a".into(), "b".into(), "c".into()]));
    app
}

fn screen(app: &App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn catalog_places_the_cursor_on_the_conversation_model() {
    assert_eq!(app().models_cursor, 1);
}

#[test]
fn arrows_move_the_cursor_within_the_catalog_and_enter_selects() {
    let mut app = app();
    app.move_model_cursor(true);
    app.move_model_cursor(true);
    assert_eq!(app.models_cursor, 2, "clamped at the last model");
    app.move_model_cursor(false);
    app.move_model_cursor(false);
    app.move_model_cursor(false);
    assert_eq!(app.models_cursor, 0, "clamped at the first model");
    app.choose_model().unwrap();
    assert_eq!(app.sessions[0].model, "a");
    assert!(!app.models_visible);
}

#[test]
fn choosing_from_an_empty_catalog_is_refused() {
    let mut app = App::new("b".into());
    app.models_visible = true;
    app.receive_models(Ok(vec![]));
    app.move_model_cursor(true);
    assert!(app.choose_model().is_err());
    assert_eq!(app.sessions[0].model, "b");
}

#[test]
fn list_highlights_the_cursor_and_names_the_keys() {
    let mut app = app();
    app.move_model_cursor(true);
    let text = screen(&app);
    assert!(text.contains("↑↓ choose · Enter select"), "{text}");
    assert!(text.lines().any(|line| line.contains("▸   c")), "{text}");
    // The conversation model stays marked separately from the cursor.
    assert!(text.lines().any(|line| line.contains("› b")), "{text}");
}

#[test]
fn cursor_stays_visible_in_a_long_catalog() {
    let mut app = App::new("m00".into());
    app.models_visible = true;
    app.receive_models(Ok((0..60).map(|n| format!("m{n:02}")).collect()));
    for _ in 0..59 {
        app.move_model_cursor(true);
    }
    assert!(screen(&app).contains("▸   m59"));
}
