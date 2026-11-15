//! Issue #728: Ctrl+N on an empty chat, the chat limit notice, `/clear` wording and the
//! `Tab chats` hint.
use alfredo_tui::{model::App, ui};
use ratatui::{backend::TestBackend, Terminal};

fn sent(app: &mut App, index: usize) {
    app.sessions[index].insert("hello");
    app.sessions[index].begin().unwrap();
}

#[test]
fn ctrl_n_on_an_empty_chat_keeps_one_chat() {
    let mut app = App::new("fixture".into());
    app.add_session();
    app.add_session();
    assert_eq!(app.sessions.len(), 1);
    assert_eq!(app.selected, 0);
    assert_eq!(
        app.notice,
        "This chat is empty; type here, or Tab to another chat"
    );
}

#[test]
fn ctrl_n_after_a_message_or_a_draft_still_adds_a_chat() {
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("a draft");
    app.add_session();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.selected, 1);
    assert!(app.notice.is_empty());

    let mut app = App::new("fixture".into());
    sent(&mut app, 0);
    app.add_session();
    assert_eq!(app.sessions.len(), 2);
    assert_eq!(app.selected, 1);
    // The new chat is empty, so another Ctrl+N is a no-op.
    app.add_session();
    assert_eq!(app.sessions.len(), 2);
}

fn eight_chats() -> App {
    let mut app = App::new("fixture".into());
    for index in 0..8 {
        if index > 0 {
            app.add_session();
        }
        app.sessions[index].insert("draft");
    }
    assert_eq!(app.sessions.len(), 8);
    app
}

#[test]
fn the_limit_notice_names_clear_and_conversation() {
    let mut app = eight_chats();
    app.add_session();
    assert_eq!(app.sessions.len(), 8);
    assert_eq!(
        app.notice,
        "Eight chats is the limit; /clear empties this chat, or start another set with --conversation NAME"
    );
}

#[test]
fn clear_with_eight_chats_does_not_promise_ctrl_n() {
    let app = eight_chats();
    let notice = app.cleared_notice();
    assert!(!notice.contains("Ctrl+N opens another"), "{notice}");
    assert!(notice.contains("eight"), "{notice}");
    let one = App::new("fixture".into());
    assert!(one.cleared_notice().contains("Ctrl+N opens another"));
}

fn title_row(app: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .find(|row| row.contains("Prompt"))
        .unwrap()
}

#[test]
fn the_prompt_title_offers_tab_only_with_two_or_more_chats() {
    let mut app = App::new("fixture".into());
    assert!(!title_row(&app, 80).contains("Tab chats"));
    sent(&mut app, 0);
    app.add_session();
    for width in [80, 120] {
        let row = title_row(&app, width);
        assert!(row.contains("Enter send") && row.contains("Tab chats"), "{row}");
    }
}
