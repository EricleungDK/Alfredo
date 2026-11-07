//! Approval-card row layout (issues #269, #303, #586): the `path · N bytes` row of a
//! file edit, and when a card is whole (armable) at a given size.
use alfredo_tui::chat_tools::approval::{Card, CardKind, FileRow};
use alfredo_tui::ui::approval_card_lines_with_file;
use ratatui::text::Line;

fn wide_card(preview_line: impl Fn(usize) -> String) -> Card {
    let preview: Vec<String> = (0..30).map(preview_line).collect();
    Card {
        tool: "write_file".into(),
        arguments: serde_json::json!({"path": "notes.md", "content": "x"}),
        kind: CardKind::Edit,
        summary: "write notes.md".into(),
        preview: preview.join("\n"),
        fingerprint: String::new(),
    }
}

/// A card with no file row: the args excerpt alone.
fn approval_card_lines(
    card: &Card,
    width: usize,
    rows: usize,
) -> (Vec<Line<'static>>, usize, bool) {
    approval_card_lines_with_file(card, None, width, rows)
}

/// A card drawn with its file row.
fn with_file(
    edit: &(Card, FileRow),
    width: usize,
    rows: usize,
) -> (Vec<Line<'static>>, usize, bool) {
    approval_card_lines_with_file(&edit.0, Some(&edit.1), width, rows)
}

fn height_of(lines: &[Line<'static>], width: usize) -> usize {
    lines
        .iter()
        .map(|l| alfredo_tui::transcript_cache::line_height(l, width as u16))
        .sum()
}

fn edit_card(path: &str, preview_lines: usize) -> (Card, FileRow) {
    let mut card = wide_card(|n| format!("+line {n}"));
    card.preview = (0..preview_lines)
        .map(|n| format!("+line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    card.arguments = serde_json::json!({"path": path, "content": "c".repeat(300)});
    let file = FileRow {
        path: path.to_string(),
        bytes: 4321,
    };
    (card, file)
}

fn texts(lines: &[Line<'static>]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

/// Issue #586: a file edit draws `path · N bytes` above the args when both fit, in
/// place of the args when only the row fits, and never when the card is not an edit.
#[test]
fn file_edit_cards_draw_a_path_and_byte_row_and_drop_args_only_when_short_of_rows() {
    let card = edit_card("src/a.rs", 3);
    let (lines, head, whole) = with_file(&card, 80, 40);
    let text = texts(&lines);
    assert!(whole && head == 3, "{text:?}");
    assert_eq!(text[2], "  src/a.rs · 4321 bytes");
    assert!(text[3].starts_with("  args "), "{text:?}");
    // Four args rows leave no room for the 3-line diff here; the row alone fits.
    let (lines, head, whole) = with_file(&card, 80, 8);
    let text = texts(&lines);
    assert!(whole && head == 2, "{text:?}");
    assert_eq!(text[2], "  src/a.rs · 4321 bytes");
    assert!(!text.iter().any(|l| l.contains("args")), "{text:?}");
    assert!(height_of(&lines, 80) <= 8);
    // Not even the row helps: deny-only keeps the old args row and wording.
    let (lines, _, whole) = with_file(&card, 80, 5);
    let text = texts(&lines);
    assert!(
        !whole && text.iter().any(|l| l.starts_with("  args ")),
        "{text:?}"
    );
    assert!(!text.iter().any(|l| l.contains("bytes")), "{text:?}");
    assert!(text.last().unwrap().contains("y is off"), "{text:?}");
    // A git card carrying a stray file row still draws args only.
    let mut git = edit_card("src/a.rs", 3);
    git.0.kind = CardKind::Git;
    let text = texts(&with_file(&git, 80, 40).0);
    assert!(!text.iter().any(|l| l.contains("bytes")), "{text:?}");
    assert!(text[2].starts_with("  args "), "{text:?}");
}

/// Issue #586: a path that wraps is never cut; the card keeps the args row. The path
/// on the row is escaped like every other card string.
#[test]
fn the_file_row_is_one_row_and_escapes_the_path() {
    let long = "p".repeat(100);
    let text = texts(&with_file(&edit_card(&long, 3), 80, 40).0);
    assert!(!text.iter().any(|l| l.contains("bytes")), "{text:?}");
    // Exactly the width: 2 + path + " · 4321 bytes" fills the row.
    let fits = "p".repeat(80 - 2 - " · 4321 bytes".chars().count());
    let text = texts(&with_file(&edit_card(&fits, 3), 80, 40).0);
    assert_eq!(text[2], format!("  {fits} · 4321 bytes"));
    let over = "p".repeat(80 - 2 - " · 4321 bytes".chars().count() + 1);
    let text = texts(&with_file(&edit_card(&over, 3), 80, 40).0);
    assert!(!text.iter().any(|l| l.contains("bytes")), "{text:?}");
    let text = texts(&with_file(&edit_card("a\u{202e}b\u{200b}.rs", 3), 80, 40).0);
    assert_eq!(text[2], "  a<U+202E>b<U+200B>.rs · 4321 bytes");
}

/// Issue #303: a very long path in the header can only arm when the header fits.
#[test]
fn edit_cards_with_very_long_paths_fit_or_stay_deny_only() {
    let path = format!("{}/file.rs", "dir/".repeat(400));
    let card = Card {
        tool: "edit_file".into(),
        arguments: serde_json::json!({"path": path, "old_text": "a", "new_text": "b"}),
        kind: CardKind::Edit,
        summary: format!("edit {path}"),
        preview: "@@\n-a\n+b".into(),
        fingerprint: String::new(),
    };
    for (width, rows) in [(80usize, 24usize), (80, 12), (80, 18)] {
        let (lines, _, whole) = approval_card_lines(&card, width, rows);
        assert!(!whole, "{width}x{rows}");
        assert!(height_of(&lines, width) <= rows, "{width}x{rows}");
        assert!(lines.last().unwrap().to_string().contains("n deny"));
    }
    let short = Card {
        summary: "edit a.rs".into(),
        ..card
    };
    let (lines, _, whole) = approval_card_lines(&short, 80, 12);
    assert!(whole && height_of(&lines, 80) <= 12);
}

/// Issue #269: every card kind arms only when every preview line is drawn.
#[test]
fn every_card_kind_is_whole_only_when_its_whole_preview_is_drawn() {
    for kind in [CardKind::Edit, CardKind::Git, CardKind::Push] {
        let card = |preview: String| Card {
            tool: "t".into(),
            arguments: serde_json::json!({}),
            kind,
            summary: "s".into(),
            preview,
            fingerprint: String::new(),
        };
        let small = card("one\ntwo".into());
        let (_, _, whole) = approval_card_lines(&small, 80, 24);
        assert!(whole, "{kind:?}: a card that fits arms");
        for preview in ["x".repeat(100_000), "a\n".repeat(60), "漢".repeat(5000)] {
            let (lines, _, whole) = approval_card_lines(&card(preview), 80, 24);
            assert!(!whole, "{kind:?}: a cut preview never arms");
            assert!(height_of(&lines, 80) <= 24, "{kind:?}");
            assert!(lines.last().unwrap().to_string().contains("n deny"));
        }
    }
}

/// Issue #586: the args excerpt is `2 x width` characters (at least 40), so a long
/// argument fills exactly that many characters of the row, not `width + 2`.
#[test]
fn the_args_excerpt_is_twice_the_width_long() {
    let card = edit_card("a.rs", 3);
    for width in [20usize, 30, 80, 100] {
        let text = texts(&with_file(&card, width, 200).0);
        let args = text.iter().find_map(|l| l.strip_prefix("  args ")).unwrap();
        assert_eq!(args.chars().count(), width.max(20) * 2, "{width}");
    }
}
