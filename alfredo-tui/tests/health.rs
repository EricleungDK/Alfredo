mod ollama_fixture;
use alfredo_tui::{
    health::{Health, HealthView, Monitor},
    model::{App, Message, Update},
    provider::Ollama,
    ui,
};
use ollama_fixture::{done, reserve, running, serve, serve_at, Reply};
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
    let fixture = serve_at(addr, |request, _| match request.path.as_str() {
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
async fn choosing_another_model_abandons_the_superseded_preload() {
    let abandoned = Arc::new(AtomicBool::new(false));
    let flag = abandoned.clone();
    let fixture = serve(reserve(), move |request, _| match request.path.as_str() {
        "GET /api/ps" => running(&[]),
        "POST /api/generate" if request.body["model"] == "first" => Reply::Hold(flag.clone()),
        "POST /api/generate" => Reply::Json(200, "{\"done\":true,\"done_reason\":\"load\"}".into()),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(40),
    );
    let view = monitor.view();
    wait_for(&view, "first", |state| *state == Health::Up).await;
    view.preload("first");
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.count("POST /api/generate") == 0 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Re-selecting the model already loading keeps its one request.
    view.preload("first");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fixture.count("POST /api/generate"), 1);
    assert!(!abandoned.load(Ordering::SeqCst));
    // The server would otherwise load every model ever picked, one by one.
    view.preload("second");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !abandoned.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "superseded preload still holds its connection"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(fixture.count("POST /api/generate"), 2);
}

#[tokio::test]
async fn dropping_the_monitor_stops_polling_and_preloads() {
    let fixture = serve(reserve(), |request, _| match request.path.as_str() {
        "GET /api/tags" => running(&["fixture"]),
        _ => running(&[]),
    });
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
            // The header names the model without its tag.
            "ollama ✓ qwen3 warm",
            "ollama ✓ warm",
        ),
        (
            Health::Loading {
                model: "qwen3:14b".into(),
            },
            "ollama ✓ qwen3 loading",
            "ollama ✓ loading",
        ),
        (Health::Up, "ollama ✓ qwen3", "ollama ✓"),
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

#[tokio::test]
async fn selected_model_missing_from_the_catalog_is_flagged_in_the_header() {
    let fixture = serve(reserve(), |request, _| match request.path.as_str() {
        "GET /api/ps" => running(&[]),
        "GET /api/tags" => running(&["qwen2.5-coder:14b"]),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(40),
    );
    let view = monitor.view();
    let state = wait_for(&view, "qwen3:14b", |state| {
        matches!(state, Health::Missing { .. })
    })
    .await;
    assert_eq!(
        state,
        Health::Missing {
            model: "qwen3:14b".into()
        }
    );
    // Installed models, including an untagged name that means `:latest`, are fine.
    assert_eq!(view.state("qwen2.5-coder:14b"), Health::Up);
    let mut app = App::new("qwen3:14b".into());
    app.health = HealthView::observed(state);
    let wide = header(&app, 140);
    assert!(
        wide.contains("model qwen3:14b not installed · /models or ollama pull qwen3:14b"),
        "{wide}"
    );
    assert!(!wide.contains("ollama ✓"), "{wide}");
    let narrow = header(&app, 80);
    assert!(narrow.contains("model not installed · /models"), "{narrow}");
    assert!(!narrow.contains("ollama ✓"), "{narrow}");
    assert_eq!(narrow.chars().count(), 80);
}

#[test]
fn model_installed_matches_exact_names_and_untagged_latest_only() {
    use alfredo_tui::provider::model_installed;
    let installed = vec!["x:latest".to_string(), "y:7b".to_string()];
    assert!(model_installed(&installed, "x"));
    assert!(model_installed(&installed, "x:latest"));
    assert!(model_installed(&installed, "y:7b"));
    assert!(!model_installed(&installed, "x:7b"));
    assert!(!model_installed(&installed, "y"));
    assert!(!model_installed(&[], "x"));
}

#[tokio::test]
async fn failing_catalog_read_never_flags_the_model_as_missing() {
    let fixture = serve(reserve(), |request, _| match request.path.as_str() {
        "GET /api/ps" => running(&[]),
        "GET /api/tags" => Reply::Json(500, "{}".into()),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(40),
    );
    let view = monitor.view();
    wait_for(&view, "qwen3:14b", |state| *state == Health::Up).await;
    let deadline = Instant::now() + Duration::from_secs(2);
    while fixture.count("GET /api/tags") < 2 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(view.state("qwen3:14b"), Health::Up);
}

#[tokio::test]
async fn bare_model_name_goes_warm_when_ollama_reports_it_as_latest() {
    let fixture = serve(reserve(), |request, _| match request.path.as_str() {
        "GET /api/ps" => running(&["qwen2.5-coder:latest"]),
        "POST /api/generate" => Reply::Json(200, "{\"done\":true,\"done_reason\":\"load\"}".into()),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(40),
    );
    let view = monitor.view();
    wait_for(&view, "qwen2.5-coder", |state| {
        matches!(state, Health::Ready { .. })
    })
    .await;
    assert_eq!(
        view.state("qwen2.5-coder:14b"),
        Health::Up,
        "another tag is not warm"
    );
}

/// Issue #659: the chat turn line draws `ctx N/WINDOW` from the live `/api/ps`
/// `context_length` of the selected model when `--num-ctx` is unset.
async fn turn_line(ps: &str, configured: Option<u32>, used: u64) -> String {
    let ps = ps.to_string();
    let fixture = serve(reserve(), move |request, _| match request.path.as_str() {
        "GET /api/ps" => Reply::Json(200, ps.clone()),
        "GET /api/tags" => running(&["fixture"]),
        _ => Reply::Json(404, "{}".into()),
    });
    let provider = Ollama::new(&fixture.endpoint, Duration::from_secs(3)).unwrap();
    let monitor = Monitor::start(
        &tokio::runtime::Handle::current(),
        provider,
        Duration::from_millis(20),
    );
    let view = monitor.view();
    wait_for(&view, "fixture", |state| !matches!(state, Health::Unknown)).await;
    // Give a second poll time to land so a context stored beside the model
    // list is visible whichever poll the wait above saw.
    tokio::time::sleep(Duration::from_millis(80)).await;
    let mut app = App::new("fixture".into());
    app.health = view;
    app.num_ctx = configured;
    app.sessions[0].insert("hello");
    app.sessions[0].begin().unwrap();
    app.sessions[0].apply(1, Update::Admitted);
    app.sessions[0].apply(1, Update::Token("answer".into()));
    app.sessions[0].apply(
        1,
        Update::Metrics(alfredo_tui::metrics::Metrics {
            prompt_eval_count: Some(used),
            ..Default::default()
        }),
    );
    app.sessions[0].apply(1, Update::Done);
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal.draw(|frame| ui::draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..30)
        .map(|y| {
            (0..120)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const PS_4096: &str = r#"{"models":[{"name":"fixture","model":"fixture","context_length":4096}]}"#;

#[tokio::test]
async fn unset_num_ctx_uses_the_observed_window() {
    let screen = turn_line(PS_4096, None, 1200).await;
    assert!(screen.contains("ctx 1200/4096"), "{screen}");
    assert!(!screen.contains("truncated"), "{screen}");
    assert!(!screen.contains("/default"), "{screen}");
}

#[tokio::test]
async fn observed_window_warns_of_truncation_near_the_limit() {
    let screen = turn_line(PS_4096, None, 3950).await;
    assert!(
        screen.contains("ctx 3950/4096 · context may be truncated"),
        "{screen}"
    );
}

#[tokio::test]
async fn unobserved_window_stays_default() {
    let no_field = r#"{"models":[{"name":"fixture","model":"fixture"}]}"#;
    let other_model = r#"{"models":[{"name":"other","model":"other","context_length":4096}]}"#;
    for ps in [no_field, other_model, r#"{"models":[]}"#] {
        let screen = turn_line(ps, None, 3950).await;
        assert!(screen.contains("ctx 3950/default"), "{ps}: {screen}");
        assert!(!screen.contains("truncated"), "{ps}: {screen}");
    }
}

#[tokio::test]
async fn configured_num_ctx_wins_over_the_observed_window() {
    let screen = turn_line(PS_4096, Some(8192), 3950).await;
    assert!(screen.contains("ctx 3950/8192"), "{screen}");
    assert!(!screen.contains("truncated"), "{screen}");
    assert!(!screen.contains("/4096"), "{screen}");
}
