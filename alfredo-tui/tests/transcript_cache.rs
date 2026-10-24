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
/// to four spaces (the golden had it vanish).
const GOLDEN: [&str; 21] = [
    "8efcb6e369f15a7356a1633cf2d2ab64b47cd8a5e7904f0ba318fa54f709656a",
    "aa80a31ced18b14400d600e7670760f10ef4d28b81c4a6315033bcf771d81daf",
    "d5f8bf37b43885cdbadf736e24e8defdd42745aa05e3a13045156383ebabe5f6",
    "735cebf0c341119efe9b47b010f92cf3d0dd8affc0b8fac32ec68297dec4be06",
    "a5b3f244a93390f75334ea4f7ac7a1a3032c576b5e22bc37a51b70339bcabf58",
    "883f3792bad3aa6dc22f39c5516a802955be399d32550955d33ef1db0a199958",
    "0d160dafacaba7ffc2020e5c7ed81af2efb3dacbedbd6f60ff35c9c2ae498c66",
    "f94667678349f12a94d8859d6e27ca3399a0ac5f91f9a65c3e713d59d16db56b",
    "c8de1b79a3e98afd3776c0cb006446365f254606c57e8ca547b9cd609dc349bf",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "74137953f371429fb1a28dbf22fb6f7223c09365871672e4372b108f3c658666",
    "faf96de0ac8d0168e9f6f51ca9a6b94d92e99ff7e2b936c393952a74606a594c",
    "829a0ee7e3da89fbac9738e8598f32d2eff9e19f9d8b57e28351ce1b902676fb",
    "8efcb6e369f15a7356a1633cf2d2ab64b47cd8a5e7904f0ba318fa54f709656a",
    "66535fad939251375f37f0614b28cacefd1b0df7c2c9ca6dc393221ce1b6ec02",
    "a39f3b71a19d1a109861a1d9e9cea3e6e680da31606d5934c69ecc1e4b78efcb",
    "f59395801898b6b88155105fab75c8a6e2cf1cb9aed3d83cff914f7514826c6e",
    "f59395801898b6b88155105fab75c8a6e2cf1cb9aed3d83cff914f7514826c6e",
    "fc7a76f58c5ad64db333173cf7f68028e989b2b65f2960ef34abbbbc95d20cde",
    "13830abb4ae1912bcd820b0a0458237835aa0b1f882f7576f6ba8959cf13a677",
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
        "x**y**z",
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
    ] {
        let app = chat(&[("user", "x"), (BODY, line)]);
        let buffer = draw_buffer(&app, 60, 12);
        let text = row_text(&buffer).join("\n");
        assert!(text.contains(shown), "{shown:?} in {text}");
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
