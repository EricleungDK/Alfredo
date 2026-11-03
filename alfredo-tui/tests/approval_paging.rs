//! Issue #485: long edit cards are paged, up to a hard cap (moved here from unit tests so
//! the mutation gate runs them).

use alfredo_tui::chat_tools::{diff, refuse_over_cap};
use alfredo_tui::ui::{approval_card_view, page_ranges};
use ratatui::text::Line;

// Issue #510: the card shows minimal hunks, one `@@ line N @@` header each.
fn hundred(changes: &[(usize, &str)]) -> (String, String) {
    let old: String = (1..=100).map(|n| format!("x{n}\n")).collect();
    let new: String = (1..=100)
        .map(|n| match changes.iter().find(|(at, _)| *at == n) {
            Some((_, text)) => format!("{text}\n"),
            None => format!("x{n}\n"),
        })
        .collect();
    (old, new)
}

fn height_of(lines: &[Line<'static>], width: usize) -> usize {
    lines
        .iter()
        .map(|l| alfredo_tui::transcript_cache::line_height(l, width as u16))
        .sum()
}

#[test]
fn a_diff_over_one_card_is_paged_up_to_the_hard_cap_and_refused_past_it() {
    // 41 scattered one-line changes: 41 headers + 82 changed lines = 123, over the
    // unpaged 80 (the card pages it) and far under the hard cap.
    let changes: Vec<(usize, &str)> = (0..41).map(|i| (2 * i + 1, "changed")).collect();
    let (old, new) = hundred(&changes);
    assert_eq!(diff(&old, &new).lines().count(), 123);
    assert!(refuse_over_cap(&old, &new, 0).is_ok());
    // A new file: header + 1999 added lines = 2000 passes; the extra line counts.
    let file = |lines: usize| (1..=lines).map(|n| format!("l{n}\n")).collect::<String>();
    assert!(refuse_over_cap("", &file(1999), 0).is_ok());
    assert!(refuse_over_cap("", &file(1999), 1).is_err());
    let error = refuse_over_cap("", &file(2000), 0).unwrap_err();
    assert!(
        error.starts_with("the change's diff is 2001 lines, over the 2000-line approval card limit; nothing was written. Split it into smaller edits"),
        "{error}"
    );
}

#[test]
fn a_change_beyond_the_diff_bound_falls_back_to_the_span_and_is_refused_past_the_cap() {
    let old: String = (0..1100).map(|n| format!("o{n}\n")).collect();
    let new: String = (0..1100).map(|n| format!("n{n}\n")).collect();
    assert!(diff(&old, &new).starts_with("@@ line 1 @@\n-o0\n"));
    assert!(refuse_over_cap(&old, &new, 0).is_err());
    // The same fallback under the cap is paged on the card, not refused.
    let (old, new) = (&old[..old.len() / 2], &new[..new.len() / 2]);
    assert!(diff(old, new).starts_with("@@ line 1 @@\n-o0\n"));
    assert!(refuse_over_cap(old, new, 0).is_ok());
}

/// Issue #485: nobody pages a headless card, so one over the unpaged 80 lines is denied.
#[test]
fn pages_hold_whole_lines_and_a_page_under_four_rows_or_a_taller_line_is_unpageable() {
    assert_eq!(
        page_ranges(&[1, 1, 1, 1, 1], 4),
        Some(vec![0..4, 4..5]),
        "a full page takes exactly its budget"
    );
    assert_eq!(page_ranges(&[2, 2, 2], 5), Some(vec![0..2, 2..3]));
    assert_eq!(page_ranges(&[1; 8], 4).map(|r| r.len()), Some(2));
    assert_eq!(page_ranges(&[1, 1, 1, 1, 1], 3), None, "under four rows");
    assert_eq!(
        page_ranges(&[1, 5, 1], 4),
        None,
        "a line taller than a page"
    );
    assert_eq!(page_ranges(&[1, 4, 1], 4), Some(vec![0..1, 1..2, 2..3]));
}

fn edit_card_of(lines: usize) -> alfredo_tui::chat_tools::approval::Card {
    use alfredo_tui::chat_tools::approval::{Card, CardKind};
    Card {
        tool: "write_file".into(),
        arguments: serde_json::json!({"path": "big.txt", "content": "x"}),
        kind: CardKind::Edit,
        summary: "new file big.txt".into(),
        preview: std::iter::once("@@ line 1 @@".to_string())
            .chain((1..=lines).map(|n| format!("+line {n}")))
            .collect::<Vec<_>>()
            .join("\n"),
        fingerprint: "big.txt:absent".into(),
    }
}

#[test]
fn a_tall_edit_card_is_paged_into_pages_that_fit_and_the_last_page_has_the_y_key() {
    let card = edit_card_of(100);
    let (width, rows) = (80usize, 20usize);
    let first = approval_card_view(&card, width, rows, 0);
    assert!(first.complete && first.pages > 1);
    assert_eq!(first.page, 0);
    let mut seen = vec![];
    for page in 0..first.pages {
        let view = approval_card_view(&card, width, rows, page);
        assert_eq!((view.page, view.pages), (page, first.pages));
        assert!(view.complete);
        assert!(height_of(&view.lines, width) <= rows, "page {page}");
        let text: Vec<String> = view.lines.iter().map(|l| l.to_string()).collect();
        seen.extend(
            text.iter()
                .filter(|l| l.starts_with("  +line"))
                .map(|l| l.trim().to_string()),
        );
        let key = text.last().unwrap();
        if page + 1 == view.pages {
            assert!(
                key.contains("y approve") && key.contains("PgUp back"),
                "{key}"
            );
        } else {
            assert!(
                key.contains("y is off") && !key.contains("y approve"),
                "{key}"
            );
        }
        assert!(
            text.iter()
                .any(|l| l.contains(&format!("[{}/{}", page + 1, view.pages))),
            "{text:?}"
        );
    }
    let expected: Vec<String> = (1..=100).map(|n| format!("+line {n}")).collect();
    assert_eq!(seen, expected, "every line on exactly one page, in order");
    // A page past the end is the last page; nothing is cut anywhere.
    let beyond = approval_card_view(&card, width, rows, 99);
    assert_eq!(beyond.page, first.pages - 1);
}

#[test]
fn a_card_that_fits_is_one_page_and_a_short_screen_or_a_git_card_is_not_paged() {
    let small = edit_card_of(3);
    let view = approval_card_view(&small, 80, 24, 0);
    assert_eq!((view.pages, view.page, view.complete), (1, 0, true));
    // Too short for a four-row page: deny-only, one "page", nothing paged.
    let view = approval_card_view(&edit_card_of(100), 80, 8, 0);
    assert_eq!((view.pages, view.complete), (1, false));
    let mut git = edit_card_of(100);
    git.kind = alfredo_tui::chat_tools::approval::CardKind::Git;
    let view = approval_card_view(&git, 80, 20, 0);
    assert_eq!((view.pages, view.complete), (1, false));
}

// Issue #485: the PgUp/PgDn and y/a-before-the-last-page decisions of the run loop.
use alfredo_tui::model::{
    arm_notice, models_scroll_step, page_direction, page_key_target, page_scroll_rows, page_step,
    ArmNotice, PageKeyTarget,
};
use crossterm::event::KeyCode;

#[test]
fn pgup_and_pgdn_step_one_page_and_clamp_at_both_ends() {
    assert_eq!(page_step(0, 4, true), 1);
    assert_eq!(page_step(2, 4, true), 3);
    assert_eq!(page_step(3, 4, true), 3, "PgDn stays on the last page");
    assert_eq!(page_step(3, 4, false), 2);
    assert_eq!(page_step(1, 4, false), 0);
    assert_eq!(page_step(0, 4, false), 0, "PgUp stays on the first page");
    assert_eq!(page_step(0, 2, true), 1);
    assert_eq!(page_step(1, 2, true), 1);
    assert_eq!(page_step(1, 2, false), 0);
}

#[test]
fn only_pgup_and_pgdn_page_and_each_moves_its_own_way() {
    assert_eq!(page_direction(KeyCode::PageUp), Some(false));
    assert_eq!(page_direction(KeyCode::PageDown), Some(true));
    for other in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Enter,
        KeyCode::Char('y'),
        KeyCode::Char('n'),
    ] {
        assert_eq!(page_direction(other), None, "{other:?}");
    }
    assert_eq!(page_scroll_rows(true), 10);
    assert_eq!(page_scroll_rows(false), -10);
    assert_eq!(models_scroll_step(0, true), 10);
    assert_eq!(models_scroll_step(25, true), 35);
    assert_eq!(models_scroll_step(25, false), 15);
    assert_eq!(models_scroll_step(5, false), 0);
    assert_eq!(models_scroll_step(u16::MAX - 3, true), u16::MAX);
}

#[test]
fn the_models_list_then_the_task_details_win_the_page_keys_before_the_chat() {
    assert_eq!(page_key_target(true, true), PageKeyTarget::Models);
    assert_eq!(page_key_target(true, false), PageKeyTarget::Models);
    assert_eq!(page_key_target(false, true), PageKeyTarget::Details);
    assert_eq!(page_key_target(false, false), PageKeyTarget::Chat);
}

#[test]
fn a_y_or_a_that_decided_nothing_names_its_reason_and_n_never_asks_for_paging() {
    for key in ['y', 'a'] {
        // Not on the last page of a paged card: page first.
        assert_eq!(arm_notice(key, false, 2), ArmNotice::PageFirst, "{key}");
        assert_eq!(arm_notice(key, false, 40), ArmNotice::PageFirst, "{key}");
        // A card that is not paged but not whole on screen: enlarge the terminal.
        assert_eq!(
            arm_notice(key, false, 1),
            ArmNotice::EnlargeTerminal,
            "{key}"
        );
        // The whole card (a single page, or the last page) is only waiting for the arm delay.
        assert_eq!(arm_notice(key, true, 1), ArmNotice::ReadFirst, "{key}");
        assert_eq!(arm_notice(key, true, 5), ArmNotice::ReadFirst, "{key}");
    }
    for whole in [false, true] {
        for pages in [1, 2, 9] {
            assert_eq!(arm_notice('n', whole, pages), ArmNotice::ReadFirst);
        }
    }
}

#[test]
fn page_key_scrolls_the_transcript_when_no_card_is_paged() {
    use alfredo_tui::model::Session;
    let mut session = Session::new("m".into());
    session.insert(&"earlier context line\n".repeat(60));
    let heights = vec![1usize; 60];
    let bottom = session.reading_position(&heights, 10);
    session.page_key(false);
    let up = session.reading_position(&heights, 10);
    assert_ne!(up, bottom, "PgUp scrolled up");
    assert!(up.line < bottom.line, "{up:?} {bottom:?}");
    session.page_key(true);
    session.page_key(true);
    let down = session.reading_position(&heights, 10);
    assert_eq!(down, bottom, "PgDn scrolled back down to the end");
}
