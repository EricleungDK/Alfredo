//! Chat transcript rendering: golden output hashes captured before the render cache
//! existed, plus the cache's reuse contract.
use alfredo_tui::{
    model::{App, Message, Status, TaskReceiptRef},
    ui,
};
use ratatui::{
    backend::TestBackend,
    style::{Color, Modifier},
    Terminal,
};
use sha2::{Digest, Sha256};

fn body(index: usize) -> String {
    match index % 6 {
        0 => format!("short answer {index}"),
        1 => format!(
            "wide line {index} {}\nsecond line with 中文宽字符 and emoji 🦀🦀 {}\n\ttabbed \x1b[31mcontrol\x07 text",
            "alpha beta gamma ".repeat(18),
            "x".repeat(150)
        ),
        2 => format!(
            "intro {index}\n```rust\nfn main() {{\n    println!(\"{}\");\n}}\n```\nafter the fence\n\n\ntrailing blanks",
            "w".repeat(140)
        ),
        3 => (0..40)
            .map(|n| format!("row {n} of message {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
        4 => "u".repeat(500),
        _ => String::new(),
    }
}

fn fixture() -> App {
    let mut app = App::new("m".into());
    for index in 0..36 {
        app.sessions[0].messages.push(Message {
            role: if index % 2 == 0 { "user" } else { "assistant" }.into(),
            content: body(index),
        });
    }
    for (sequence, after) in [(0, 0), (1, 4), (2, 6), (3, 20), (4, 36)] {
        assert!(app.sessions[0].observe_task_receipt(TaskReceiptRef {
            sequence,
            after_messages: after,
            revision: 1 + sequence,
            task: 7,
            correlation: format!("golden-{sequence}"),
        }));
    }
    app
}

fn frame(terminal: &mut Terminal<TestBackend>, app: &App) -> String {
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    format!(
        "{:x}",
        Sha256::digest(format!("{:?}", terminal.backend().buffer()).as_bytes())
    )
}

/// One scripted session of redraws: follow tail, scrolling, resizes, streaming, failure.
fn scenario() -> Vec<String> {
    let mut app = fixture();
    let mut hashes = vec![];
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    hashes.push(frame(&mut terminal, &app));
    for rows in [-20, -57, -300, 11] {
        app.sessions[0].scroll_rows(rows);
        hashes.push(frame(&mut terminal, &app));
    }
    for (width, height) in [(60, 20), (140, 40), (24, 9), (100, 30)] {
        terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        hashes.push(frame(&mut terminal, &app));
        app.sessions[0].scroll_rows(-7);
        hashes.push(frame(&mut terminal, &app));
    }
    app.sessions[0].scroll_rows(-5000);
    hashes.push(frame(&mut terminal, &app));
    app.sessions[0].scroll_rows(100_000);
    hashes.push(frame(&mut terminal, &app));
    for token in [" streamed", " tokens\nnew line ", &"z".repeat(300)] {
        app.sessions[0]
            .messages
            .last_mut()
            .unwrap()
            .content
            .push_str(token);
        hashes.push(frame(&mut terminal, &app));
    }
    app.sessions[0].messages[3].content.push_str(" edited");
    hashes.push(frame(&mut terminal, &app));
    app.sessions[0].status = Status::Failed("boom".into());
    hashes.push(frame(&mut terminal, &app));
    app.sessions[0].messages.truncate(10);
    hashes.push(frame(&mut terminal, &app));
    hashes
}

/// Captured on the uncached renderer (origin/main @ 52e5e90) before the cache existed;
/// re-captured for #258, which adds the `^O expand results` footer hint (frame bodies
/// are unchanged); re-captured for #404, which expands the tab in message 1's `\ttabbed` line
/// to four spaces (the golden had it vanish);
/// re-captured for #399, which adds the `Ctrl+R retries` footer hint to the two failed-turn frames;
/// re-captured for #393, which widens the chat footer's scroll hint to `PgUp/Dn/Home/End scroll`
/// (the changed frames are the wide ones that show that hint; bodies are unchanged);
/// re-captured for #468 and #436: ESC and BEL in message 1 now show as `<U+001B>` and
/// `<U+0007>` (controls are shown as code points), in the three frames that show that line;
/// re-captured for #568: the side pane's first chat row gains a dim `short answer 0` preview line
/// (only that row changes, in the 17 frames wide enough to show the side pane).
const GOLDEN: [&str; 21] = [
    "a372316e8a86236d0e5a9c8ecd95895352c3915f17f0e429ec61ce03d23b3f61",
    "d199951b19fb0fac070bcbca5682890b06bce4de4abf16669224743969985d15",
    "3f8ee5543cec2035f9509cf41ede7cf05ad819be4b599102301dae2bef708b6c",
    "3dd5b1e2e3548b65f2db65869d1b1cac4d0ebbd309c970ae6bdf9f0ceadfd52d",
    "671c5b397567670f23e7152675f584977ec95c3de83f5ff1e4168785f5a924a6",
    "883f3792bad3aa6dc22f39c5516a802955be399d32550955d33ef1db0a199958",
    "0d160dafacaba7ffc2020e5c7ed81af2efb3dacbedbd6f60ff35c9c2ae498c66",
    "dd680b7ab8005762f116843d009a373c868bf0b1e5d95584b3e708a2e18d3c3e",
    "9430fca1dc296936bc0812a8fa3e1aad9b32726fd40bf1af341d7fc42556c210",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "94a085c378a471f3f0e010932872ac2e7c409f972e765b1d49edc95985c30960",
    "bd042d47387912b591becc6b32f154d873882ba7bd51e3ca35890a704146f91e",
    "679fba24512e275b859d2985d71a2650e28976466e2ed89d11f43b34dc0d4841",
    "a372316e8a86236d0e5a9c8ecd95895352c3915f17f0e429ec61ce03d23b3f61",
    "d0cf77937bf28de4c846a9b5c2a3c99ed6c011a130cc9c15bb3c3a2ab1ddcc1d",
    "2917066a782e19d0b45ce34f9e608cf110a32269391455f7d174f0a9bdc25bb2",
    "95d93183212f3cae8a8ad9d2afcc2625764523c0d8e16d9ae2e206b93e35742c",
    "95d93183212f3cae8a8ad9d2afcc2625764523c0d8e16d9ae2e206b93e35742c",
    "e82f417de56f8fe12a4633f1b9f9241abe486ec209cdb96210213963b0a5fd66",
    "95b4916a73dc1d7b5f2a9429018523cbc152aa51a64a41a329ef6ffb9d7e8eea",
];

#[test]
fn transcript_output_matches_the_uncached_renderer() {
    let hashes = scenario();
    assert_eq!(hashes, GOLDEN);
}

fn chat(messages: &[(&str, &str)]) -> App {
    let mut app = App::new("m".into());
    for (role, content) in messages {
        app.sessions[0].messages.push(Message {
            role: (*role).into(),
            content: (*content).into(),
        });
    }
    app
}

fn draw_buffer(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &ratatui::buffer::Buffer) -> Vec<Vec<String>> {
    let area = buffer.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect()
        })
        .collect()
}

fn row_text(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    rows(buffer).into_iter().map(|r| r.concat()).collect()
}

/// The cell of the first symbol of `needle` in the first row containing `needle`.
fn cell_of<'a>(buffer: &'a ratatui::buffer::Buffer, needle: &str) -> &'a ratatui::buffer::Cell {
    let first = needle.chars().next().unwrap().to_string();
    for (y, row) in rows(buffer).iter().enumerate() {
        if row.concat().contains(needle) {
            let mut x = 0;
            while x < row.len() {
                if row[x..].concat().starts_with(needle) && row[x] == first {
                    return &buffer[(x as u16, y as u16)];
                }
                x += 1;
            }
        }
    }
    panic!("{needle:?} not drawn in {:#?}", row_text(buffer));
}

const BODY: &str = "assistant";

#[test]
fn fenced_code_is_styled_markers_dimmed_and_lines_cut_not_wrapped() {
    let long = "Z".repeat(120);
    let app = chat(&[
        ("user", "show code"),
        (
            BODY,
            &format!("intro\n```rust\nlet x = 1;\n{long}\n```\nafter"),
        ),
    ]);
    let buffer = draw_buffer(&app, 40, 30);
    assert_ne!(cell_of(&buffer, "let x").fg, cell_of(&buffer, "intro").fg);
    assert_eq!(cell_of(&buffer, "```rust").fg, Color::DarkGray);
    let per_row: Vec<usize> = row_text(&buffer)
        .iter()
        .map(|r| r.matches('Z').count())
        .collect();
    assert_eq!(
        per_row.iter().filter(|n| **n > 0).count(),
        1,
        "one row, not wrapped: {per_row:?}"
    );
    assert!(per_row.iter().all(|n| *n < 40));
    assert!(row_text(&buffer).iter().any(|r| r.contains("after")));
    let cut_row = row_text(&buffer)
        .into_iter()
        .find(|r| r.contains('Z'))
        .unwrap();
    assert!(
        cut_row
            .trim_end_matches('\u{2502}')
            .trim_end()
            .ends_with('\u{2026}'),
        "cut row ends in the marker: {cut_row:?}"
    );
    assert_eq!(cut_row.matches('\u{2026}').count(), 1);
}

#[test]
fn rows_that_fit_get_no_cut_marker() {
    let app = chat(&[("user", "x"), (BODY, "```\nshort line\n```")]);
    let text = row_text(&draw_buffer(&app, 40, 20)).join("\n");
    assert!(text.contains("short line"));
    assert!(!text.contains('\u{2026}'), "{text}");
}

#[test]
fn cut_row_with_wide_char_at_the_boundary_still_ends_in_the_marker() {
    use alfredo_tui::transcript_cache::cut_line;
    use ratatui::text::{Line, Span};
    let line = Line::from(vec![Span::raw("ab"), Span::raw("\u{4e2d}\u{6587}cd")]);
    // width 5: "ab" + one wide char (4) leaves 1 column, which the marker takes.
    let cut = cut_line(&line, 5);
    let text: String = cut.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "ab\u{4e2d}\u{2026}");
    assert_eq!(cut.width(), 5);
    // width 4: the wide char would straddle the marker column, so it is dropped and
    // the marker is padded into the last column.
    let cut = cut_line(&line, 4);
    let text: String = cut.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, "ab \u{2026}");
    assert_eq!(cut.width(), 4);
    // A line that fits is returned unchanged.
    let fits = cut_line(&line, 8);
    assert_eq!(fits.width(), 8);
    assert!(!fits.spans.iter().any(|s| s.content.contains('\u{2026}')));
}

#[test]
fn asterisks_that_are_not_bold_render_verbatim() {
    let lines = [
        "Matches src/**/*.rs and tests/**/*.rs here; also 2 ** 3 and 4 ** 5.",
        "Use 2 ** 3 and 4 ** 5 in python, and def f(**kw, **o).",
        "a lone ** here and a trailing **",
        "***both***",
        "2**3 and 4**5",
    ];
    for line in lines {
        let app = chat(&[("user", "x"), (BODY, line)]);
        let buffer = draw_buffer(&app, 100, 12);
        let text = row_text(&buffer).join("\n");
        assert!(text.contains(line), "{line:?} kept verbatim in {text}");
    }
}

#[test]
fn flanked_bold_still_consumes_its_markers() {
    for (line, bold, shown) in [
        ("so **bold** here", "bold", "so bold here"),
        ("**Note:** careful", "Note:", "Note: careful"),
        ("see **a `b` c** end", "a ", "see a `b` c end"),
        ("(**x**)", "x", "(x)"),
        (
            "**\u{4e2d}\u{6587}**\u{7684}",
            "\u{4e2d}",
            "\u{4e2d}\u{6587}\u{7684}",
        ),
        (
            "\u{8fd9}\u{662f}**\u{91cd}\u{70b9}**",
            "\u{91cd}",
            "\u{8fd9}\u{662f}\u{91cd}\u{70b9}",
        ),
        ("**bold**text", "bold", "boldtext"),
        ("**Note**: x", "Note", "Note: x"),
        ("**bold**. y", "bold", "bold. y"),
        ("**foo**'s y", "foo", "foo's y"),
        ("- **Item**: text", "Item", "- Item: text"),
        ("**1.** one", "1.", "1. one"),
    ] {
        let app = chat(&[("user", "x"), (BODY, line)]);
        let buffer = draw_buffer(&app, 60, 12);
        let text = row_text(&buffer).join("\n");
        // Wide characters leave a skipped cell in the TestBackend buffer.
        let squash = |t: &str| t.replace(' ', "");
        assert!(
            squash(&text).contains(&squash(shown)),
            "{shown:?} in {text}"
        );
        assert!(!text.contains("**"), "{text}");
        assert!(cell_of(&buffer, bold).modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn headings_bold_inline_code_and_bullets_are_styled() {
    let app = chat(&[
        ("user", "x"),
        (
            BODY,
            "# Title\nsome **strong** and `code` words\n- first\n* second\n1. third",
        ),
    ]);
    let buffer = draw_buffer(&app, 60, 30);
    let text = row_text(&buffer).join("\n");
    assert!(cell_of(&buffer, "Title").modifier.contains(Modifier::BOLD));
    assert!(cell_of(&buffer, "strong").modifier.contains(Modifier::BOLD));
    assert!(!cell_of(&buffer, "some").modifier.contains(Modifier::BOLD));
    assert!(!text.contains("**"), "balanced bold markers are consumed");
    assert_ne!(
        cell_of(&buffer, "code words").fg,
        cell_of(&buffer, "some").fg
    );
    for marker in ["- first", "* second", "1. third"] {
        let cell = cell_of(&buffer, marker);
        assert_ne!(
            cell.fg,
            cell_of(&buffer, "some").fg,
            "{marker} marker styled"
        );
    }
}

#[test]
fn unbalanced_or_unknown_markup_stays_plain() {
    let app = chat(&[
        ("user", "x"),
        (
            BODY,
            "a **never closed\nb `open tick\n#nospace\nc * lone * star\nd ***",
        ),
    ]);
    let buffer = draw_buffer(&app, 60, 30);
    let text = row_text(&buffer).join("\n");
    for raw in [
        "**never closed",
        "`open tick",
        "#nospace",
        "* lone * star",
        "d ***",
    ] {
        assert!(text.contains(raw), "{raw:?} kept verbatim in {text}");
    }
    assert!(!cell_of(&buffer, "never").modifier.contains(Modifier::BOLD));
}

#[test]
fn unterminated_fence_renders_as_an_open_block() {
    let mut app = chat(&[("user", "x"), (BODY, "text\n```py\nprint(1)")]);
    let buffer = draw_buffer(&app, 50, 30);
    assert_ne!(cell_of(&buffer, "print").fg, cell_of(&buffer, "text").fg);
    // Streaming more of the block keeps it styled and closing it ends the block.
    app.sessions[0].messages[1]
        .content
        .push_str("\n```\nback to prose");
    let buffer = draw_buffer(&app, 50, 30);
    assert_ne!(cell_of(&buffer, "print").fg, cell_of(&buffer, "text").fg);
    assert_eq!(cell_of(&buffer, "back").fg, cell_of(&buffer, "text").fg);
}

#[test]
fn model_text_never_reaches_the_buffer_raw() {
    let evil = "\x1b[31mred\u{202E}evil\x07";
    let app = chat(&[
        ("user", "x"),
        (
            BODY,
            &format!("```\ncode {evil}\n```\nthen **bold {evil}** end `tick {evil}`"),
        ),
    ]);
    let buffer = draw_buffer(&app, 80, 30);
    for row in rows(&buffer) {
        for symbol in row {
            assert!(
                !symbol.chars().any(|c| c.is_control() || c == '\u{202E}'),
                "raw symbol {symbol:?} in the buffer"
            );
        }
    }
    let text = row_text(&buffer).join("\n");
    assert!(text.contains("[31mred"), "ESC dropped, text kept");
    assert!(
        text.contains("red<U+202E>evil"),
        "override shown, not drawn"
    );
}

#[test]
fn user_messages_stay_verbatim() {
    let app = chat(&[("user", "# h **b** `c`\n- item"), (BODY, "ok")]);
    let buffer = draw_buffer(&app, 60, 30);
    let text = row_text(&buffer).join("\n");
    assert!(
        text.contains("# h **b** `c`") && text.contains("- item"),
        "{text}"
    );
    assert!(!cell_of(&buffer, "**b**").modifier.contains(Modifier::BOLD));
    assert_eq!(cell_of(&buffer, "# h").fg, cell_of(&buffer, "- item").fg);
}

#[test]
fn an_unchanged_markdown_message_is_not_reparsed() {
    let app = chat(&[("user", "x"), (BODY, "# T\n```\ncode\n```\n**b**")]);
    draw_buffer(&app, 60, 30);
    let first = app.sessions[0].transcript_cache_stats();
    assert_eq!(first.builds, 2);
    draw_buffer(&app, 60, 30);
    assert_eq!(app.sessions[0].transcript_cache_stats(), first);
}

#[test]
fn streaming_a_token_rebuilds_only_the_last_message() {
    let mut app = fixture();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    frame(&mut terminal, &app);
    let first = app.sessions[0].transcript_cache_stats();
    assert_eq!(first.builds, 36, "cold draw builds each message once");

    // An unchanged frame reuses every message.
    frame(&mut terminal, &app);
    assert_eq!(app.sessions[0].transcript_cache_stats(), first);

    // A streamed token touches only the last message.
    app.sessions[0]
        .messages
        .last_mut()
        .unwrap()
        .content
        .push_str(" token");
    frame(&mut terminal, &app);
    let after = app.sessions[0].transcript_cache_stats();
    assert_eq!(after.builds, first.builds + 1);
    assert_eq!(after.height_passes, first.height_passes + 1);

    // A resize re-measures cached bodies without rebuilding them.
    terminal = Terminal::new(TestBackend::new(70, 30)).unwrap();
    frame(&mut terminal, &app);
    let resized = app.sessions[0].transcript_cache_stats();
    assert_eq!(resized.builds, after.builds);
    assert_eq!(resized.height_passes, after.height_passes + 36);
}

/// Issue #436: tag characters, zero-width marks and line/paragraph separators in a reply
/// show as `<U+XXXX>` in prose and inside fenced code, as the approval card shows them.
#[test]
fn invisible_code_points_in_replies_show_as_code_points() {
    let prose = "ok\u{E0041}\u{E0042}\u{2028}done \u{200B}x **bold\u{200B}** `t\u{E0041}`";
    let fenced = "```\ncode\u{E0041}\u{200B}\u{2028}end\n```";
    let app = chat(&[("user", "x"), (BODY, &format!("{prose}\n{fenced}"))]);
    let buffer = draw_buffer(&app, 80, 30);
    for row in rows(&buffer) {
        for symbol in row {
            assert!(
                !symbol
                    .chars()
                    .any(|c| matches!(c, '\u{E0041}' | '\u{E0042}' | '\u{200B}' | '\u{2028}')),
                "raw symbol {symbol:?} in the buffer"
            );
        }
    }
    let text = row_text(&buffer).join("\n");
    assert!(text.contains("ok<U+E0041><U+E0042><U+2028>done"), "{text}");
    assert!(text.contains("code<U+E0041><U+200B><U+2028>end"), "{text}");
    assert!(text.contains("<U+200B>x"), "{text}");
}

/// Issue #436: a `<U+XXXX>` marker next to `**` keeps the flanking rules, so the markers
/// still pair and the text between them is bold.
#[test]
fn hidden_characters_beside_bold_markers_keep_the_bold() {
    for (line, shown, bold) in [
        ("**bold\u{200B}** end", "bold<U+200B> end", "bold"),
        ("\u{E0041}**bold**", "<U+E0041>bold", "bold"),
        ("see **qz\u{2028}wq** end", "see qz<U+2028>wq end", "qz"),
    ] {
        let app = chat(&[("user", "x"), (BODY, line)]);
        let buffer = draw_buffer(&app, 60, 12);
        let text = row_text(&buffer).join("\n");
        assert!(text.contains(shown), "{line:?} shows {text}");
        assert!(!text.contains("**"), "{line:?} kept its markers: {text}");
        let cell = cell_of(&buffer, bold);
        assert!(
            cell.modifier.contains(Modifier::BOLD),
            "{line:?}: {bold:?} not bold"
        );
    }
}

/// Issue #436 (guard): a user draft is shown through the same helper as replies, and CJK,
/// accents, emoji and tab expansion must render exactly as before.
#[test]
fn a_user_draft_with_cjk_accents_emoji_and_tabs_renders_unchanged() {
    let app = chat(&[("user", "你好 café 🙂\n\tx"), (BODY, "ok")]);
    let buffer = draw_buffer(&app, 60, 30);
    let text = row_text(&buffer).join("\n");
    assert!(text.contains('你') && text.contains("café 🙂"), "{text}");
    assert!(
        text.contains("    x"),
        "tab expanded to the next stop: {text}"
    );
    assert!(
        !text.contains("<U+"),
        "nothing escaped in plain text: {text}"
    );
}

/// Issue #436 (decision): in a reply an emoji zero-width joiner and a variation selector show
/// as `<U+200D>` and `<U+FE0F>`, as on the approval card; plain emoji, accents and CJK are
/// unchanged.
#[test]
fn reply_emoji_joiners_and_selectors_show_as_code_points() {
    let app = chat(&[(
        "user",
        "x",
    ), (BODY, "fam \u{1F468}\u{200D}\u{1F469} heart \u{2764}\u{FE0F} smile \u{1F642} caf\u{E9} \u{4F60}")]);
    let buffer = draw_buffer(&app, 80, 30);
    let text = row_text(&buffer).join("\n");
    assert!(text.contains("<U+200D>"), "{text}");
    assert!(text.contains("<U+FE0F>"), "{text}");
    assert!(
        text.contains('\u{1F642}') && text.contains("caf\u{E9}") && text.contains('\u{4F60}'),
        "{text}"
    );
    assert!(
        !text.contains("\u{200D}") && !text.contains("\u{FE0F}"),
        "{text}"
    );
}
