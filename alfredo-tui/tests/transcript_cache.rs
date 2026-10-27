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
/// (the changed frames are the wide ones that show that hint; bodies are unchanged).
const GOLDEN: [&str; 21] = [
    "79e33b494ca7e0b0cb7efaab742759b360f99eee302a32277dc3072722b42b53",
    "d10d31cdc917e429a66da765248b31f13eed56bef302b23ce67adeef890a560a",
    "f4fafc9f1f3e032579c6f39e5bdd69fe8a4f7ea7eaf0dc90d92dca15efaa3a6b",
    "57025740de7158f351fc48ac47ee16e091fd625e7c1020554611edc06092dbe9",
    "5e820ea5757ea8491a6a75ab4de22053f3b14227e90a3ef0e034cda1182010aa",
    "883f3792bad3aa6dc22f39c5516a802955be399d32550955d33ef1db0a199958",
    "0d160dafacaba7ffc2020e5c7ed81af2efb3dacbedbd6f60ff35c9c2ae498c66",
    "8fadde1e14d4d87cda5adb5720c30b88dd90e5184f8193a96d0e348f5e32f95a",
    "36846bf02582e75bf51e282bab99b4e75844967430d9f7507aeff9fea813fbd7",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "bfbbefefaaff5d19f25d0c8765b289619655e24648378a3206a689644a63fb39",
    "aa6a34899781549760aa6bf14e88ea43b7f79b3584069a44a8c9b324360e241c",
    "88fdba48d3cad1878c13c2b654f1e4303189df075d2b4c5e1dfa020e4a73184f",
    "79e33b494ca7e0b0cb7efaab742759b360f99eee302a32277dc3072722b42b53",
    "67e61da96f70b321c2c04f48f2143c92268b82e19dff3bc4ed6d271c7eac2474",
    "27cbd2754954363982606a6a6fe9482ad1ac544d75073539149b60ffdf608378",
    "4c3814088036ee7f49c1322fc883a5fd1ce6653307274002a3de4840e65fbe3b",
    "4c3814088036ee7f49c1322fc883a5fd1ce6653307274002a3de4840e65fbe3b",
    "5bafd1e125329425b4a9c1807e2280ebededfc87aba146c0711418b687f56a32",
    "c2492680b2b10d894c98a43c5d6d5558d7280d28142d58409a846d49b93611e8",
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
