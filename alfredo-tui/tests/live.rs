use alfredo_tui::{
    model::{Message, Update},
    provider::Ollama,
};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[tokio::test]
#[ignore = "Requires a running Ollama server and installed local model"]
async fn live_ollama_completes_through_the_rust_transport() {
    let endpoint = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let model = std::env::var("ALFREDO_SMOKE_MODEL").unwrap_or_else(|_| "qwen2.5-coder:14b".into());
    let provider = Ollama::new(&endpoint, Duration::from_secs(60)).unwrap();
    let (sender, mut receiver) = mpsc::channel(128);
    let start = Instant::now();
    let job = tokio::spawn(async move {
        provider
            .chat(
                0,
                1,
                model,
                vec![Message {
                    role: "user".into(),
                    content: "Reply with only the word READY.".into(),
                }],
                sender,
            )
            .await;
    });
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        let mut text = String::new();
        let mut first_token = None;
        let mut thinking_started = None;
        while let Some(event) = receiver.recv().await {
            match event.update {
                Update::Metrics(metrics) => println!("{}", metrics.summary()),
                Update::Queued
                | Update::QueueProgress(_)
                | Update::CapacityWait { .. }
                | Update::Admitted
                | Update::Retrying(_) => {}
                Update::Thinking => { thinking_started.get_or_insert_with(|| start.elapsed()); }
                Update::Token(token) => {
                    first_token.get_or_insert_with(|| start.elapsed());
                    text.push_str(&token);
                }
                Update::Done => {
                    assert!(!text.trim().is_empty());
                    println!(
                        "live smoke: thinking_started={thinking_started:?}, first_content={first_token:?}, complete={:?}, reply={text:?}",
                        start.elapsed()
                    );
                    return;
                }
                Update::Failed(error) => panic!("Live provider failed: {error}"),
            }
        }
        panic!("Provider closed without completion");
    })
    .await;
    job.abort();
    result.expect("Live inference exceeded 90 seconds");
}

#[tokio::test]
#[ignore = "Requires a running Ollama server and installed local model"]
async fn live_preload_keeps_the_model_warm_for_the_next_prompt() {
    let endpoint = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let model = std::env::var("ALFREDO_SMOKE_MODEL").unwrap_or_else(|_| "qwen2.5-coder:14b".into());
    let provider = Ollama::new(&endpoint, Duration::from_secs(60))
        .unwrap()
        .with_keep_alive(Some("30m".into()));
    let start = Instant::now();
    provider.preload(&model).await.unwrap();
    println!("live preload: {:?}", start.elapsed());
    assert!(provider.running_models().await.unwrap().contains(&model));
    let ps: serde_json::Value = reqwest::get(format!("{endpoint}/api/ps"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let expires = ps["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == model.as_str())
        .unwrap()["expires_at"]
        .clone();
    println!("live keep_alive: expires_at={expires}");
    let (sender, mut receiver) = mpsc::channel(128);
    let start = Instant::now();
    let job = tokio::spawn(async move {
        provider
            .chat(
                0,
                1,
                model,
                vec![Message {
                    role: "user".into(),
                    content: "Reply with only the word READY.".into(),
                }],
                sender,
            )
            .await;
    });
    let mut first = None;
    while let Some(event) = receiver.recv().await {
        match event.update {
            Update::Token(_) => {
                first.get_or_insert_with(|| start.elapsed());
            }
            Update::Metrics(metrics) => println!("live warm metrics: {}", metrics.summary()),
            Update::Done => break,
            Update::Failed(error) => panic!("Live provider failed: {error}"),
            _ => {}
        }
    }
    job.abort();
    println!(
        "live warm prompt: first_content={first:?}, complete={:?}",
        start.elapsed()
    );
    assert!(first.is_some());
}
