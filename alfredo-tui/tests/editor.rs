use alfredo_tui::model::{App, Session, MAX_DRAFT};
use unicode_width::UnicodeWidthStr;

#[test]
fn edits_in_the_middle_preserve_emoji_clusters_and_combining_marks() {
    let mut editor = Session::new("fixture".into());
    editor.insert("Ae\u{301}👩‍💻界Z");
    editor.home();
    editor.right();
    editor.right();
    assert_eq!(editor.cursor(), "Ae\u{301}".len());
    editor.delete();
    assert_eq!(editor.draft, "Ae\u{301}界Z");
    editor.backspace();
    assert_eq!(editor.draft, "A界Z");
    editor.insert("🦀");
    assert_eq!(editor.draft, "A🦀界Z");
    editor.left();
    editor.delete();
    assert_eq!(editor.draft, "A界Z");
    editor.end();
    editor.backspace();
    assert_eq!(editor.draft, "A界");
}

#[test]
fn cursor_remains_visible_in_cell_bounded_views_at_both_ends_and_middle() {
    let mut editor = Session::new("fixture".into());
    editor.insert(&"界👩‍💻e\u{301} ".repeat(100));
    for width in [1, 2, 7, 24, 80] {
        for position in [0, 30, 400] {
            editor.home();
            for _ in 0..position {
                editor.right();
            }
            let visible = editor.draft_view(width);
            assert!(visible.contains('▏'));
            assert!(visible.width() <= width, "{visible:?}");
            assert!(!visible.starts_with('\u{301}'));
        }
    }
    editor.home();
    assert!(editor.draft_view(20).starts_with('▏'));
    editor.end();
    assert!(editor.draft_view(20).ends_with('▏'));
}

#[test]
fn paste_is_bounded_without_splitting_a_cluster_and_commands_reset_the_cursor() {
    let mut editor = Session::new("fixture".into());
    editor.insert(&"x".repeat(MAX_DRAFT - 5));
    editor.insert("👩‍💻");
    assert_eq!(editor.draft.len(), MAX_DRAFT - 5);
    editor.clear_draft();
    editor.insert("first second  ");
    editor.delete_word();
    assert_eq!(editor.draft, "first ");
    editor.home();
    editor.insert("\x1b\x07new\n");
    assert_eq!(editor.draft, "new\nfirst ");
    assert!(editor.draft_view(80).contains('↵'));
    editor.begin().unwrap();
    assert_eq!(editor.cursor(), 0);
    assert!(editor.draft.is_empty());
}

#[test]
fn session_switch_keeps_independent_drafts_and_cursor_positions() {
    let mut app = App::new("fixture".into());
    app.sessions[0].insert("ac");
    app.sessions[0].left();
    app.add_session();
    app.sessions[1].insert("second");
    app.sessions[0].insert("b");
    assert_eq!(app.sessions[0].draft, "abc");
    assert_eq!(app.sessions[1].draft, "second");
    assert_eq!(app.sessions[0].cursor(), 2);
    assert_eq!(app.sessions[1].cursor(), 6);
}

#[test]
fn paste_keeps_tab_indentation_as_four_spaces_and_the_cursor_follows_them() {
    let mut editor = Session::new("fixture".into());
    let result = editor.insert("fn x() {\n\treturn 1;\r\n}\n");
    assert_eq!(editor.draft, "fn x() {\n    return 1;\n}\n");
    assert_eq!(result.accepted, editor.draft.len());
    assert!(!result.truncated);
    let mut editor = Session::new("fixture".into());
    editor.insert("a");
    editor.insert("b");
    editor.home();
    editor.right();
    editor.insert("\t");
    assert_eq!(editor.draft, "a    b");
    assert_eq!(editor.cursor(), 5);
    editor.insert("a\tb\x1b\x07");
    assert_eq!(editor.draft, "a    a    bb");
}

#[test]
fn insert_reports_truncation_only_when_the_limit_cut_text() {
    let mut editor = Session::new("fixture".into());
    let cut = editor.insert(&"a".repeat(MAX_DRAFT + 10));
    assert_eq!(cut.accepted, MAX_DRAFT);
    assert!(cut.truncated);
    assert_eq!(editor.draft.len(), MAX_DRAFT);
    editor.clear_draft();
    let exact = editor.insert(&"a".repeat(MAX_DRAFT));
    assert!(!exact.truncated);
    // Tab expansion counts against the limit.
    editor.clear_draft();
    editor.insert(&"a".repeat(MAX_DRAFT - 3));
    let tab = editor.insert("\t");
    assert_eq!((tab.accepted, tab.truncated), (0, true));
    assert_eq!(editor.draft.len(), MAX_DRAFT - 3);
}
