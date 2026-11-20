//! Issue #745: on a short terminal (a tmux half-split of 36 rows is 18) a one-line
//! `run_command` card must still be answerable, and a short pane must still show the
//! last line of the transcript.
use alfredo_tui::{
    chat_tools::approval::{self, Decision},
    model::{App, Session, ToolCall, Update},
    ui,
};
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

static ID: AtomicU64 = AtomicU64::new(0);

fn workspace() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "alfredo-short-{}-{}",
        std::process::id(),
        ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("sub")).unwrap();
    root
}

fn shell_session(arguments: serde_json::Value) -> Session {
    let root = workspace();
    let call = ToolCall {
        name: "run_command".into(),
        arguments,
    };
    let card = approval::card(&root, &call).unwrap();
    let _ = fs::remove_dir_all(root);
    let mut session = Session::new("m".into());
    session.insert("run it");
    session.begin().unwrap();
    let attempt = session.attempt;
    session.apply(attempt, Update::Admitted);
    assert!(session.await_approval(attempt, card, Instant::now()));
    session
}

fn screen(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// What a drawn run_command card answers to `y`, `a` then `n` once armed.
struct Probe {
    y: Option<Decision>,
    a: Option<Decision>,
    n: Option<Decision>,
    whole: bool,
    text: String,
}

fn probe(session: Session, width: u16, height: u16) -> Probe {
    let mut app = App::new("m".into());
    app.sessions[0] = session;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let start = Instant::now();
    let mut draw = |app: &mut App, now: Instant| {
        terminal.draw(|frame| ui::draw(frame, app)).unwrap();
        let session = &mut app.sessions[0];
        let drawn = session.approval_header_drawn();
        session.approval_visible(drawn, now);
        screen(&terminal)
    };
    draw(&mut app, start);
    let later = start + approval::ARM * 2;
    let text = draw(&mut app, later);
    let session = &mut app.sessions[0];
    let whole = session.approval_whole();
    let at = later + approval::ARM;
    Probe {
        y: session.answer_approval('y', at),
        a: session.answer_approval('a', at),
        n: session.answer_approval('n', at),
        whole,
        text,
    }
}

/// The ticket's repro: `run_command touch pwned` at 80x18 said `y is off`.
#[test]
fn a_one_line_run_command_card_is_approvable_at_80x18() {
    for (width, height) in [(80, 18), (120, 18), (80, 19), (80, 24)] {
        let probe = probe(shell_session(json!({"command": "touch pwned"})), width, height);
        let text = &probe.text;
        assert!(probe.whole, "{width}x{height}: {text}");
        assert!(text.contains("$ touch pwned"), "{width}x{height}: {text}");
        assert!(text.contains("y approve"), "{width}x{height}: {text}");
        assert_eq!(probe.y, Some(Decision::Approve), "{width}x{height}: {text}");
    }
}

/// A host run is armable at 80x18 too, and stays red-labelled and never offers `a`.
#[test]
fn a_host_run_command_card_is_approvable_at_80x18_and_keeps_its_label() {
    let probe = probe(
        shell_session(json!({"command": "touch pwned", "escalate": true})),
        80,
        18,
    );
    let text = &probe.text;
    assert!(text.contains("! HOST"), "{text}");
    assert!(text.contains("run on HOST"), "{text}");
    assert!(text.contains("$ touch pwned"), "{text}");
    assert_eq!(probe.a, None, "{text}");
    assert_eq!(probe.y, Some(Decision::Approve), "{text}");
}

/// The ticket's second symptom: at 10 rows the chat pane showed no transcript.
#[test]
fn a_one_line_reply_is_on_screen_at_every_short_height() {
    for height in 10..=14u16 {
        let mut app = App::new("m".into());
        app.sessions[0].insert("hello there");
        app.sessions[0].begin().unwrap();
        app.sessions[0].apply(1, Update::Token("REPLYTEXT".into()));
        app.sessions[0].apply(1, Update::Done);
        let mut terminal = Terminal::new(TestBackend::new(80, height)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
        let text = screen(&terminal);
        assert!(text.contains("REPLYTEXT"), "80x{height}: {text}");
    }
}
