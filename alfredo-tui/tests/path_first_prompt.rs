// Issue #744: a prompt whose first token is a file path (`/etc/hosts what is this`) is chat
// text, not a slash command; real command names never contain a second `/`.
use alfredo_tui::{
    commands::{is_command_token, Completion},
    model::Session,
};

#[test]
fn a_first_token_with_a_second_slash_is_not_a_command() {
    for text in [
        "/etc/hosts what is this",
        "/usr/bin/env what is this",
        "/tmp/x.py",
        "//go",
        "  /etc/hosts",
        "/etc/",
        "/home/me/proj/src/lib.rs has a bug in parse()",
    ] {
        assert!(!is_command_token(text), "{text}");
    }
}

#[test]
fn real_and_mistyped_commands_stay_commands() {
    for text in [
        "/go X",
        "/tasks",
        "/allow",
        "/gp fix it",
        "/clea",
        "/",
        "  /clear",
        "/go fix /etc/hosts",
        "/go\tX",
    ] {
        assert!(is_command_token(text), "{text}");
    }
    for text in ["", "hello /etc", "etc/hosts", "@wayfinder /x", " "] {
        assert!(!is_command_token(text), "{text}");
    }
}

fn running() -> Session {
    let mut session = Session::new("test-model".into());
    session.insert("Explain this repository");
    session.begin().unwrap();
    session
}

#[test]
fn a_path_first_prompt_is_queued_like_any_chat_text_during_a_turn() {
    let mut session = running();
    session.insert("/etc/hosts what is this");
    session.queue_draft().unwrap();
    assert_eq!(session.queued_messages(), ["/etc/hosts what is this"]);
    assert_eq!(session.draft, "");
}

#[test]
fn a_mistyped_command_is_still_refused_by_the_queue_and_keeps_its_text() {
    for draft in ["/gp fix it", "/clea"] {
        let mut session = running();
        session.insert(draft);
        assert!(session.queue_draft().unwrap_err().contains("Commands"));
        assert_eq!(session.draft, draft);
        assert!(session.queued_messages().is_empty());
    }
}

#[test]
fn completion_does_not_accept_a_path_first_draft() {
    for draft in ["/etc/", "/etc/hosts", "//go", "/usr/bin/env"] {
        assert!(!Completion::accepts(draft), "{draft}");
        assert!(Completion::typeahead(draft).is_none(), "{draft}");
        assert!(Completion::open(draft).is_none(), "{draft}");
    }
}

#[test]
fn completion_still_accepts_commands_mentions_and_model_arguments() {
    for draft in ["/", "/go", "/clea", "@wayfinder", "@", "/model ", "/model ll"] {
        assert!(Completion::accepts(draft), "{draft}");
    }
    for draft in ["", "hello", "/go X", "/etc/hosts what", "@wayfinder x"] {
        assert!(!Completion::accepts(draft), "{draft}");
    }
    assert!(Completion::typeahead("/clea").is_some());
    assert!(Completion::typeahead("/").is_some());
    assert!(Completion::typeahead("/clear x").is_none());
    assert!(Completion::open("/clea").is_some());
    assert!(Completion::open("/").is_some());
    assert!(Completion::open("@").is_some());
    assert!(Completion::open("/clear x").is_none());
}
