//! One wake source for the terminal run loop.
//!
//! The loop used to `event::poll(33 ms)` and look at model, planner, worker and
//! task results only after that poll timed out, so every result reached the
//! screen up to one poll period late. Now the loop blocks on one channel that
//! carries two kinds of signal:
//!
//! * terminal input, forwarded by the single reader thread that is the only
//!   caller of `crossterm::event::{poll, read}` while the loop is active (an
//!   in-loop prompt such as the workspace selector reads through [`Input`] too);
//! * a coalesced "something finished" wake raised whenever a Tokio worker of the
//!   application runtime is about to park. Every producer of loop-visible state
//!   (provider streams, planner, task control, worker progress, health) is a task
//!   on that runtime, so a send always precedes the park that raises the wake
//!   and no sender needs to know about the loop. Blocking jobs the loop polls with
//!   `is_finished` go through [`spawn_blocking`], which completes on a worker.
//!
//! The loop keeps a timeout so spinner and elapsed-time redraws still run
//! without input; an idle terminal does no more work than before.
use crossterm::event::{self, Event};
use std::{
    cell::Cell,
    io,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, task::JoinHandle};

/// How long the reader thread blocks in `poll` before it re-checks its stop flag.
const READER_SLICE: Duration = Duration::from_millis(250);

/// Wake passes allowed back to back, then the sustained spacing between them.
const PACE_BURST: f64 = 8.0;
const PACE_INTERVAL: Duration = Duration::from_millis(20);

enum Signal {
    Input(Event),
    Wake,
    Failed(io::ErrorKind, String),
}

/// What the loop learned from one wait.
#[derive(Debug)]
pub enum Next {
    Event(Event),
    /// A runtime worker went idle: re-read every channel and task.
    Woken,
    Timeout,
}

/// Hang-up and terminate requests received by the interactive process (issue #342).
static HANGUP: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_hangup(signum: libc::c_int) {
    // The first signal asks for an orderly quit; a second one means the quit is stuck,
    // so it gets the default action (end the process) like before the handler existed.
    if HANGUP.fetch_add(1, Ordering::SeqCst) >= 1 {
        // SAFETY: signal and raise are async-signal-safe.
        unsafe {
            libc::signal(signum, libc::SIG_DFL);
            libc::raise(signum);
        }
    }
}

/// Make SIGHUP (closed terminal, dropped ssh) and SIGTERM request a quit like Ctrl+Q
/// instead of ending the process at once, which would orphan a running host command.
/// Interactive run loop only; the loop sees the flag within its 33 ms wait timeout.
pub fn catch_hangup() {
    let handler = on_hangup as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGHUP, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

/// Whether SIGHUP or SIGTERM arrived since [`catch_hangup`].
pub fn hangup_requested() -> bool {
    HANGUP.load(Ordering::SeqCst) > 0
}

/// A source of terminal events, so in-loop prompts do not race the reader thread.
pub trait InputSource {
    fn next_event(&mut self, timeout: Duration) -> io::Result<Option<Event>>;
}

/// Reads the terminal directly; only valid while no reader thread is running.
pub struct Direct;
impl InputSource for Direct {
    fn next_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if event::poll(timeout)? {
            event::read().map(Some)
        } else {
            Ok(None)
        }
    }
}

struct Shared {
    sender: Sender<Signal>,
    wake_queued: AtomicBool,
    stop: AtomicBool,
}
impl Shared {
    fn wake(&self) {
        if !self.wake_queued.swap(true, Ordering::AcqRel) {
            let _ = self.sender.send(Signal::Wake);
        }
    }
}

pub struct Input {
    receiver: Receiver<Signal>,
    shared: Arc<Shared>,
    pace: Cell<(f64, Instant)>,
}
impl Input {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            receiver,
            pace: Cell::new((PACE_BURST, Instant::now())),
            shared: Arc::new(Shared {
                sender,
                wake_queued: AtomicBool::new(false),
                stop: AtomicBool::new(false),
            }),
        }
    }

    /// The application runtime. A worker about to park has finished whatever it
    /// was running, including sends to the loop's channels, so it wakes the loop.
    pub fn runtime(&self) -> io::Result<Runtime> {
        let shared = self.shared.clone();
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .on_thread_park(move || shared.wake())
            .build()
    }

    /// Start the one thread that reads terminal events. Call once, right before
    /// the loop; nothing else may call `event::poll` or `event::read` until the
    /// `Input` is dropped (use the `Input` itself as the [`InputSource`]).
    pub fn spawn_reader(&self) -> io::Result<()> {
        let shared = self.shared.clone();
        thread::Builder::new()
            .name("alfredo-input".into())
            .spawn(move || {
                while !shared.stop.load(Ordering::Acquire) {
                    let signal = match event::poll(READER_SLICE) {
                        Ok(false) => continue,
                        Ok(true) => match event::read() {
                            Ok(event) => Signal::Input(event),
                            Err(error) => Signal::Failed(error.kind(), error.to_string()),
                        },
                        Err(error) => Signal::Failed(error.kind(), error.to_string()),
                    };
                    let failed = matches!(signal, Signal::Failed(..));
                    if shared.sender.send(signal).is_err() || failed {
                        break;
                    }
                }
            })
            .map(|_| ())
    }

    /// Raise the wake from outside the runtime (tests and non-runtime producers).
    pub fn waker(&self) -> Waker {
        Waker(self.shared.clone())
    }

    /// Block until terminal input, a wake or the timeout.
    pub fn wait(&self, timeout: Duration) -> io::Result<Next> {
        match self.receiver.recv_timeout(timeout) {
            Ok(signal) => self.settle(signal),
            Err(RecvTimeoutError::Timeout) => Ok(Next::Timeout),
            Err(RecvTimeoutError::Disconnected) => {
                Err(io::Error::other("terminal input thread stopped"))
            }
        }
    }

    /// Like [`Input::wait`], but wakes are paced by a token bucket: a burst of
    /// `PACE_BURST` passes is immediate (first-chunk and reply latency), then
    /// one pass per `PACE_INTERVAL`, so a fast stream costs about the CPU of the
    /// old 33 ms poll. Keys are never delayed and a held wake is never lost.
    pub fn wait_paced(&self, timeout: Duration) -> io::Result<Next> {
        match self.wait(timeout)? {
            Next::Woken => {}
            other => return Ok(other),
        }
        let (mut tokens, at) = self.pace.get();
        let now = Instant::now();
        tokens = (tokens + now.duration_since(at).as_secs_f64() / PACE_INTERVAL.as_secs_f64())
            .min(PACE_BURST);
        if tokens < 1.0 {
            let deadline = now + PACE_INTERVAL.mul_f64(1.0 - tokens);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                match self.wait(left)? {
                    Next::Event(event) => {
                        self.pace.set((tokens, now));
                        return Ok(Next::Event(event));
                    }
                    Next::Woken | Next::Timeout => {}
                }
            }
            tokens = 1.0;
        }
        self.pace.set((tokens - 1.0, Instant::now()));
        Ok(Next::Woken)
    }

    fn settle(&self, signal: Signal) -> io::Result<Next> {
        match signal {
            Signal::Input(event) => Ok(Next::Event(event)),
            // Clear before the caller reads its channels: a send after this
            // point raises a new wake instead of being swallowed by this one.
            Signal::Wake => {
                self.shared.wake_queued.store(false, Ordering::Release);
                Ok(Next::Woken)
            }
            Signal::Failed(kind, message) => Err(io::Error::new(kind, message)),
        }
    }
}
impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
impl InputSource for Input {
    /// Terminal events only; wakes are dropped because the loop re-reads every
    /// source when the prompt returns.
    fn next_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.wait(left)? {
                Next::Event(event) => return Ok(Some(event)),
                Next::Woken if !left.is_zero() => {}
                Next::Woken | Next::Timeout => return Ok(None),
            }
        }
    }
}

#[derive(Clone)]
pub struct Waker(Arc<Shared>);
impl Waker {
    pub fn wake(&self) {
        self.0.wake();
    }
}

/// `Runtime::spawn_blocking` for jobs the loop polls with `is_finished`: the
/// job completes on a runtime worker, whose next park wakes the loop after the
/// handle reads finished (a bare blocking-pool thread would wake it too early).
pub fn spawn_blocking<R, F>(runtime: &Runtime, job: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    runtime.spawn(async move {
        match tokio::task::spawn_blocking(job).await {
            Ok(value) => value,
            Err(error) => match error.try_into_panic() {
                Ok(panic) => std::panic::resume_unwind(panic),
                Err(error) => panic!("blocking job cancelled: {error}"),
            },
        }
    })
}
