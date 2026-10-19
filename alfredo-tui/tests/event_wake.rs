//! The run loop's wake source: runtime activity wakes it without a key press.
//! Waits use a generous timeout and assert which signal arrived, never how fast.
use alfredo_tui::wake::{spawn_blocking, Input, Next};
use std::time::Duration;

const LONG: Duration = Duration::from_secs(20);

fn woken(input: &Input) -> bool {
    matches!(input.wait(LONG).unwrap(), Next::Woken)
}

#[test]
fn channel_event_sent_by_a_runtime_task_wakes_the_loop() {
    let input = Input::new();
    let runtime = input.runtime().unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        sender.send("streamed chunk").await.unwrap();
    });
    // Parks before the send are harmless extra wakes; the send must produce one.
    let chunk = loop {
        assert!(woken(&input), "no wake before the long timeout");
        if let Ok(chunk) = receiver.try_recv() {
            break chunk;
        }
    };
    assert_eq!(chunk, "streamed chunk");
}

#[test]
fn every_event_in_a_stream_wakes_the_loop() {
    let input = Input::new();
    let runtime = input.runtime().unwrap();
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    runtime.spawn(async move {
        for chunk in 0..20 {
            sender.send(chunk).await.unwrap();
        }
    });
    let mut seen = 0;
    while seen < 20 {
        assert!(woken(&input));
        while let Ok(chunk) = receiver.try_recv() {
            assert_eq!(chunk, seen);
            seen += 1;
        }
    }
}

#[test]
fn finished_blocking_job_is_finished_when_the_wake_arrives() {
    let input = Input::new();
    let runtime = input.runtime().unwrap();
    for round in 0..100 {
        let job = spawn_blocking(&runtime, move || round);
        loop {
            assert!(woken(&input));
            if job.is_finished() {
                break;
            }
        }
        assert_eq!(runtime.block_on(job).unwrap(), round);
    }
}

#[test]
fn quiet_runtime_times_out_and_explicit_wake_is_coalesced() {
    let input = Input::new();
    assert!(matches!(
        input.wait(Duration::from_millis(20)).unwrap(),
        Next::Timeout
    ));
    let waker = input.waker();
    waker.wake();
    waker.wake();
    assert!(woken(&input));
    assert!(matches!(
        input.wait(Duration::from_millis(20)).unwrap(),
        Next::Timeout
    ));
    waker.wake();
    assert!(woken(&input));
}

#[test]
fn paced_wait_allows_a_burst_then_spaces_wakes_without_losing_one() {
    use std::time::Instant;
    let input = Input::new();
    let waker = input.waker();
    let started = Instant::now();
    // The burst is delivered at once: each wake is a pass, none is dropped.
    for _ in 0..8 {
        waker.wake();
        assert!(matches!(input.wait_paced(LONG).unwrap(), Next::Woken));
    }
    // Past the burst a wake is held for the spacing, then still delivered.
    waker.wake();
    assert!(matches!(input.wait_paced(LONG).unwrap(), Next::Woken));
    assert!(started.elapsed() < Duration::from_secs(10));
    // Nothing pending: times out.
    assert!(matches!(
        input.wait_paced(Duration::from_millis(60)).unwrap(),
        Next::Timeout
    ));
}
