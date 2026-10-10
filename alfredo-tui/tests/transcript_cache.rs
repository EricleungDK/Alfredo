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

/// Captured on the uncached renderer (origin/main @ 52e5e90) before the cache existed.
const GOLDEN: [&str; 21] = [
    "ca60a861669a160060a51d584d5a4f43b728c14fe12cc8769a61072faede36e3",
    "806c35dcb905601a5f7cbb104e05702fb931a33d973ed7700cad48d336469607",
    "7f7da7f139ee9bc99f6de0de6912055f01577afeed0a07e66eda17ae35b8a0e4",
    "35118aaf0c113d937dca9c842d48997a2c351a96139a8a7ce3682c103f41322d",
    "8f8346d25f926f2d21a6e57d2ef57d2209a011d0568356ad824eabe0b090dd01",
    "7aff67f9c8b878506289fc08aa8146069e9ddf7d865f5426b9c3484d03b307d9",
    "5617ffbea49611a9a25264dc98c4525b76a89bcb36456d6ea8d8a5cacc9eb1e1",
    "28ba1956c954e26e3693466d623660e382669c5c80302665ef0eff95285c5cb0",
    "71a782275cda7602701d733814f0210e69cf11a154be8e044aa44f02af07ec30",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "fc4ca5b9376d952cbcf62f5e9fbb371c58c6e087223afd22f605894cf92a6cd7",
    "8d382cf8b4be504fa06935e93074984821e8c10bec00b8ff1c5d25fcea1a6279",
    "7daf1213238366893bffe9ed3872111f185089770bec70559a397dc8a4585624",
    "08643a8334051b0297f831c55cee931cef9f6fc7f56e24ff9e5500110c977b12",
    "ca60a861669a160060a51d584d5a4f43b728c14fe12cc8769a61072faede36e3",
    "1286607772acfa8d6e4acf405824f615cc0ba36218dfa963b90ddf0bf5ee10d6",
    "a28f63de86538ff4c59898971afb0f0ce62bd504b8ed1b618302edf6d0657810",
    "0e6af0b5cec8a5617504e8bce634219816b1489af16e78658ac4af51b93e3285",
    "0e6af0b5cec8a5617504e8bce634219816b1489af16e78658ac4af51b93e3285",
    "255e84a4827b71b1513d49dc290321d269dc993556cf864eb4779483b7a4b0a7",
    "a73a3348f065bd0af3e5fc02a9634f0d5ff0f804409672c9c839c3bfda2f815e",
];

#[test]
fn transcript_output_matches_the_uncached_renderer() {
    let hashes = scenario();
    assert_eq!(hashes, GOLDEN);
}
