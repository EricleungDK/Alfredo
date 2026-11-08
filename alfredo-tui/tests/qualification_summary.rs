use alfredo_tui::qualification;
use std::{
    fs,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[tokio::test]
async fn summary_of_an_incomplete_cohort_names_why_it_stopped() {
    let dir = std::env::temp_dir().join(format!(
        "alfredo-qualification-summary-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir(&dir).unwrap();
    let report = tokio::time::timeout(
        Duration::from_secs(30),
        qualification::run(
            &dir.join("q.json"),
            "http://127.0.0.1:9",
            "m",
            1,
            Some(false),
            Arc::new(AtomicBool::new(false)),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let reason = report
        .stop_reason
        .clone()
        .expect("unreachable endpoint stops the cohort");
    let summary = report.summary();
    let _ = fs::remove_dir_all(&dir);
    let mut cleared = report.clone();
    cleared.stop_reason = None;
    let without = cleared.summary();
    assert!(!without.contains("Stopped:"), "{without}");
    assert_eq!(without.lines().count(), 1, "{without}");
    let mut lines = summary.lines();
    assert!(lines
        .next()
        .unwrap()
        .starts_with("Diagnostic cohort incomplete"));
    assert_eq!(
        lines.next(),
        Some(format!("Stopped: {reason}").as_str()),
        "{summary}"
    );
}
