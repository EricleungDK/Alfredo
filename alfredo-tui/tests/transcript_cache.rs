//! Chat transcript rendering: golden output hashes captured before the render cache
//! existed, plus the cache's reuse contract.
use alfredo_tui::{
    model::{App, Message, Status, TaskReceiptRef},
    ui,
};
use ratatui::{backend::TestBackend, Terminal};
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
/// are unchanged).
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
    "01d06276ba96ec1db89995339d834d8aaca05a262a5e75d2f9129e91f497e324",
    "0ea73612074aea3fab34eb068f1907760475670f8be46ba930da4cdec7f3d2b4",
    "0d9eb1cf232eb7ed8194cfef7fdba5779c3e4f1febfc4f5b41a1ef64df2965d3",
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
