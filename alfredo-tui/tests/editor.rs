use alfredo_tui::model::{App, Session, MAX_DRAFT};
use alfredo_tui::ui;
use ratatui::{backend::TestBackend, Terminal};
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
fn multi_line_draft_window_shows_each_line_with_the_caret_and_keeps_wide_cells_aligned() {
    let mut editor = Session::new("fixture".into());
    editor.insert("one\n界👩‍💻e\u{301}x\nthree");
    assert_eq!(editor.draft_line_count(), 3);
    assert_eq!(
        editor.draft_lines(20, 6),
        ["one", "界👩‍💻e\u{301}x", "three▏"]
    );
    // Cursor on the middle line, after the wide character.
    editor.home();
    editor.right();
    editor.right();
    editor.right();
    editor.right();
    editor.right();
    assert_eq!(
        editor.draft_lines(20, 6),
        ["one", "界▏👩‍💻e\u{301}x", "three"]
    );
    // Narrow widths never overflow in cells and keep the caret.
    for width in [1, 2, 3, 5] {
        for row in editor.draft_lines(width, 6) {
            assert!(row.width() <= width, "{row:?}");
        }
        assert!(editor
            .draft_lines(width, 6)
            .iter()
            .any(|row| row.contains('▏')));
    }
    // A single-line draft is untouched: one row, same as draft_view.
    editor.clear_draft();
    editor.insert("plain 界");
    assert_eq!(editor.draft_line_count(), 1);
    assert_eq!(editor.draft_lines(20, 6), [editor.draft_view(20)]);
}

#[test]
fn multi_line_draft_window_scrolls_to_keep_the_cursor_line_visible() {
    let mut editor = Session::new("fixture".into());
    editor.insert("l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7");
    assert_eq!(editor.draft_lines(10, 3), ["l5", "l6", "l7▏"]);
    editor.home();
    assert_eq!(editor.draft_lines(10, 3), ["▏l0", "l1", "l2"]);
    for _ in 0..(3 * 4) {
        editor.right();
    }
    // Cursor at the start of l4: the window ends on it.
    assert_eq!(editor.draft_lines(10, 3), ["l2", "l3", "▏l4"]);
}

#[test]
fn up_and_down_move_between_draft_lines_and_only_report_false_at_the_edges() {
    let mut editor = Session::new("fixture".into());
    editor.insert("ab界\ncd\nefghij");
    editor.end();
    assert!(editor.line_up());
    // Column is measured in cells: the cursor lands at the end of the short line.
    assert_eq!(&editor.draft[..editor.cursor()], "ab界\ncd");
    assert!(editor.line_up());
    assert_eq!(&editor.draft[..editor.cursor()], "ab");
    assert!(!editor.line_up());
    assert_eq!(&editor.draft[..editor.cursor()], "ab");
    assert!(editor.line_down());
    assert_eq!(&editor.draft[..editor.cursor()], "ab界\ncd");
    assert!(editor.line_down());
    assert!(!editor.line_down());
    // A single-line draft always falls through to history.
    editor.clear_draft();
    editor.insert("solo");
    assert!(!editor.line_up());
    assert!(!editor.line_down());
}

fn screen(app: &App, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn prompt_box_grows_with_the_draft_up_to_six_rows_and_the_transcript_gives_way() {
    let mut app = App::new("fixture".into());
    let before = screen(&app, 80, 24);
    // Single-line draft: three-row box, one text row, exactly as before.
    app.sessions[0].insert("only");
    let single = screen(&app, 80, 24);
    assert!(single[24 - 4].contains("only▏"), "{single:#?}");
    assert!(single[24 - 5].contains('┌') || single[24 - 5].contains('╭'));
    assert_eq!(before[..24 - 4], single[..24 - 4]);
    app.sessions[0].clear_draft();
    app.sessions[0].insert("one\n界two\nthree");
    let grown = screen(&app, 80, 24);
    assert!(grown[24 - 6].contains("one"), "{grown:#?}");
    assert!(grown[24 - 5].contains('界'));
    assert!(grown[24 - 4].contains("three▏"));
    assert!(grown[24 - 7].contains('┌') || grown[24 - 7].contains('╭'));
    assert!(!grown.iter().any(|row| row.contains('↵')));
    // Many lines: capped at six text rows, scrolled to the cursor line.
    app.sessions[0].clear_draft();
    app.sessions[0].insert(
        &(0..10)
            .map(|n| format!("row{n}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let capped = screen(&app, 80, 24);
    assert!(capped[24 - 4].contains("row9▏"));
    assert!(capped[24 - 9].contains("row4"));
    assert!(!capped.iter().any(|row| row.contains("row3")));
    // Small terminals still render with the grown box.
    let small = screen(&app, 50, 14);
    assert!(small.iter().any(|row| row.contains("row9▏")));
    let tiny = screen(&app, 32, 10);
    assert!(tiny.iter().any(|row| row.contains("row9▏")));
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
fn word_motions_stop_at_whitespace_boundaries_and_keep_clusters_whole() {
    let mut editor = Session::new("fixture".into());
    editor.insert("hello  wor\u{301}ld 👩‍💻x\nnext");
    editor.word_left();
    assert_eq!(&editor.draft[editor.cursor()..], "next");
    editor.word_left();
    assert_eq!(&editor.draft[editor.cursor()..], "👩‍💻x\nnext");
    editor.word_left();
    assert_eq!(&editor.draft[editor.cursor()..], "wor\u{301}ld 👩‍💻x\nnext");
    editor.word_left();
    assert_eq!(editor.cursor(), 0);
    editor.word_left();
    assert_eq!(editor.cursor(), 0);
    editor.word_right();
    assert_eq!(&editor.draft[..editor.cursor()], "hello");
    editor.word_right();
    assert_eq!(&editor.draft[..editor.cursor()], "hello  wor\u{301}ld");
    editor.word_right();
    assert_eq!(&editor.draft[..editor.cursor()], "hello  wor\u{301}ld 👩‍💻x");
    editor.word_right();
    editor.word_right();
    assert_eq!(editor.cursor(), editor.draft.len());
}

#[test]
fn kill_to_end_deletes_from_the_cursor_and_is_a_noop_at_the_end() {
    let mut editor = Session::new("fixture".into());
    editor.insert("keep 👩‍💻 drop\nme");
    editor.end();
    editor.kill_to_end();
    assert_eq!(editor.draft, "keep 👩‍💻 drop\nme");
    editor.home();
    editor.word_right();
    editor.kill_to_end();
    assert_eq!(editor.draft, "keep");
    assert_eq!(editor.cursor(), "keep".len());
    editor.insert("!");
    assert_eq!(editor.draft, "keep!");
}

#[test]
fn newline_insert_respects_the_draft_limit_and_motions_clear_a_rejection() {
    let mut editor = Session::new("fixture".into());
    editor.insert("abc");
    editor.insert("\n");
    editor.insert("def");
    assert_eq!(editor.draft, "abc\ndef");
    assert!(editor.draft_view(80).contains('↵'));
    editor.clear_draft();
    editor.insert(&"x".repeat(MAX_DRAFT));
    assert!(editor.insert("\n").truncated);
    assert_eq!(editor.draft.len(), MAX_DRAFT);
    for edit in 0..3 {
        let mut session = Session::new("fixture".into());
        session.insert("/gp x");
        session.reject_draft();
        match edit {
            0 => session.word_left(),
            1 => session.word_right(),
            _ => session.kill_to_end(),
        }
        assert!(!session.replace_pending(), "edit {edit}");
    }
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

#[test]
fn a_multibyte_character_straddling_the_limit_is_dropped_whole_and_lone_cr_is_stripped() {
    let mut editor = Session::new("fixture".into());
    editor.insert(&"a".repeat(MAX_DRAFT - 3));
    let result = editor.insert("🦀");
    assert_eq!((result.accepted, result.truncated), (0, true));
    assert_eq!(editor.draft.len(), MAX_DRAFT - 3);
    assert!(editor.draft.chars().all(|c| c == 'a'));
    editor.clear_draft();
    let result = editor.insert("a\rb\r\nc");
    assert_eq!(editor.draft, "ab\nc");
    assert!(!result.truncated);
}

#[test]
fn rejected_command_is_replaced_by_the_next_typed_character_and_recalled_with_up() {
    let mut session = Session::new("fixture".into());
    session.insert("/go");
    session.reject_draft();
    assert!(session.replace_pending());
    assert_eq!(session.draft, "/go");
    session.type_char('/');
    assert!(!session.replace_pending());
    assert_eq!(session.draft, "/");
    session.insert("go Make answer return 42");
    assert_eq!(session.draft, "/go Make answer return 42");

    let mut session = Session::new("fixture".into());
    session.insert("/help");
    session.reject_draft();
    session.clear_draft();
    session.history_previous();
    assert_eq!(session.draft, "/help");
}

#[test]
fn editing_or_moving_keys_keep_the_rejected_text_and_edit_it_normally() {
    for edit in 0..5 {
        let mut session = Session::new("fixture".into());
        session.insert("/gp x");
        session.reject_draft();
        match edit {
            0 => session.left(),
            1 => session.home(),
            2 => session.end(),
            3 => session.backspace(),
            _ => session.delete_word(),
        }
        assert!(!session.replace_pending(), "edit {edit}");
        session.type_char('!');
        assert!(
            session.draft.contains('!') && session.draft.contains('/'),
            "edit {edit}"
        );
    }
}

#[test]
fn first_up_after_a_rejection_keeps_the_text_and_the_next_up_goes_older() {
    let mut session = Session::new("fixture".into());
    session.insert("/older");
    session.remember_submission();
    session.clear_draft();
    session.insert("/go");
    session.reject_draft();
    session.history_previous();
    assert!(!session.replace_pending());
    assert_eq!(session.draft, "/go");
    session.history_previous();
    assert_eq!(session.draft, "/older");
    session.history_next();
    assert_eq!(session.draft, "/go");
}

#[test]
fn pasting_over_a_rejected_command_replaces_it() {
    let mut session = Session::new("fixture".into());
    session.insert("/go");
    session.reject_draft();
    // The paste arm of the event loop performs exactly this sequence.
    if session.replace_pending() {
        session.clear_draft();
    }
    session.insert("/go build x");
    assert_eq!(session.draft, "/go build x");
}
