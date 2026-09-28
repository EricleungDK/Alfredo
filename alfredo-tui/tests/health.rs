mod ollama_fixture;
use alfredo_tui::{
    health::{Health, HealthView, Monitor},
    model::{App, Message, Update},
    provider::Ollama,
    ui,
};
use ollama_fixture::{done, reserve, running, serve, Reply};
use ratatui::{backend::TestBackend, Terminal};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

async fn wait_for(view: &HealthView, model: &str, accept: impl Fn(&Health) -> bool) -> Health {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = view.state(model);
        if accept(&state) {
            return state;
        }
        assert!(Instant::now() < deadline, "last state {state:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn server_restart_mid_session_recovers_request_and_health_without_restart() {
    let addr = reserve();
    let endpoint = ollama_fixture::endpoint(addr);
    let provider = Ollama::new(&endpoint, Duration::from_secs(3))
        .unwrap()
        .with_connect_retries(5)
        .unwrap()
        .with_retry_backoff(Duration::from_millis(150));
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider.clone(),
        Duration::from_millis(40),
    );
    let view = monitor.view();
    assert!(matches!(
        wait_for(&view, "fixture", |state| matches!(
            state,
            Health::Down { .. }
        ))
        .await,
        Health::Down { .. }
    ));
    let (sender, mut receiver) = mpsc::channel(64);
    let chat = tokio::spawn(async move {
        provider
            .chat(
                0,
                1,
                "fixture".into(),
                vec![Message {
                    role: "user".into(),
                    content: "hello".into(),
                }],
                sender,
            )
            .await;
    });
    let first = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first.update,
        Update::Admitted | Update::Retrying(_)
    ));
    let fixture = serve(addr, |request, _| match request.path.as_str() {
        "GET /api/ps" => running(&["fixture"]),
        "POST /api/chat" => done("back"),
        _ => Reply::Json(404, "{}".into()),
    });
    chat.await.unwrap();
    let mut events = vec![first];
    while let Some(event) = receiver.recv().await {
        events.push(event);
    }
    assert!(events
        .iter()
        .any(|event| matches!(event.update, Update::Retrying(_))));
    assert!(matches!(events.last().unwrap().update, Update::Done));
    assert_eq!(
        wait_for(&view, "fixture", |state| matches!(
            state,
            Health::Ready { .. }
        ))
        .await,
        Health::Ready {
            model: "fixture".into()
        }
    );
    assert_eq!(view.state("other"), Health::Up);
    assert!(fixture.count("GET /api/ps") >= 1);
}

#[tokio::test]
async fn preload_reports_loading_then_warm_and_failure_is_only_status() {
    let loaded = Arc::new(AtomicBool::new(false));
    let flag = loaded.clone();
    let fixture = serve(reserve(), move |request, _| match request.path.as_str() {
        "GET /api/ps" if flag.load(Ordering::SeqCst) => running(&["fixture"]),
        "GET /api/ps" => running(&[]),
        "POST /api/generate" if request.body["model"] == "fixture" => {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::SeqCst);
            Reply::Json(200, "{\"done\":true,\"done_reason\":\"load\"}".into())
        }
        "POST /api/generate" => Reply::Json(500, "{\"error\":\"no memory\"}".into()),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(40),
    );
    let view = monitor.view();
    wait_for(&view, "fixture", |state| *state == Health::Up).await;
    // `/model NAME` selection warms the chosen model without blocking.
    let mut app = App::new("other".into());
    app.health = view.clone();
    app.models = vec!["fixture".into()];
    let started = Instant::now();
    app.select_model("fixture").unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(
        view.state("fixture"),
        Health::Loading {
            model: "fixture".into()
        }
    );
    wait_for(&view, "fixture", |state| {
        matches!(state, Health::Ready { .. })
    })
    .await;
    assert!(view.preload_error().is_none());
    view.preload("broken");
    let deadline = Instant::now() + Duration::from_secs(5);
    while view.preload_error().is_none() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(view.preload_error().unwrap().contains("500"));
    assert_eq!(view.state("broken"), Health::Up);
}

#[tokio::test]
async fn dropping_the_monitor_stops_polling_and_preloads() {
    let fixture = serve(reserve(), |_, _| running(&[]));
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(20),
    );
    let view = monitor.view();
    wait_for(&view, "fixture", |state| *state == Health::Up).await;
    assert!(!view.stopped());
    drop(monitor);
    assert!(view.stopped());
    tokio::time::sleep(Duration::from_millis(50)).await;
    let polls = fixture.count("GET /api/ps");
    view.preload("fixture");
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(fixture.count("GET /api/ps"), polls);
    assert_eq!(fixture.count("POST /api/generate"), 0);
}

#[test]
fn changes_are_observed_once_for_redraw() {
    let view = HealthView::observed(Health::Up);
    assert!(view.changed());
    assert!(!view.changed());
    assert!(!HealthView::default().changed());
}

fn header(app: &App, width: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..width)
        .map(|x| buffer[(x, 0)].symbol().to_string())
        .collect::<String>()
}

#[test]
fn header_renders_server_health_at_narrow_and_wide_widths() {
    let mut app = App::new("qwen3:14b".into());
    assert!(!header(&app, 140).contains("ollama"));
    for (state, wide, narrow) in [
        (
            Health::Ready {
                model: "qwen3:14b".into(),
            },
            "ollama ✓ qwen3:14b warm",
            "ollama ✓ warm",
        ),
        (
            Health::Loading {
                model: "qwen3:14b".into(),
            },
            "ollama ✓ qwen3:14b loading",
            "ollama ✓ loading",
        ),
        (Health::Up, "ollama ✓ qwen3:14b", "ollama ✓"),
        (
            Health::Down {
                since: Instant::now(),
            },
            "ollama ✗ retrying",
            "ollama ✗ retrying",
        ),
    ] {
        app.health = HealthView::observed(state);
        let line = header(&app, 140);
        assert!(line.contains(wide), "{line}");
        assert!(line.contains("ALFREDO"));
        let line = header(&app, 80);
        assert!(line.contains(narrow), "{line}");
        assert!(line.contains("ALFREDO"));
        assert_eq!(line.chars().count(), 80);
    }
}
