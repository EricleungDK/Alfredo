//! Time and cancel budget of the read-tier calls of one tool round (issue #257).
//!
//! A blocking tool cannot be interrupted by aborting its future, so the round carries
//! a [`Budget`]: a wall deadline and a cancel flag set when the turn is dropped. The
//! executor sets it on the blocking thread ([`within`]); `grep` and the git readers
//! look at it ([`current`]) between files and while waiting on git.
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Wall time one tool round may take before the calls not reached are skipped.
pub const ROUND_LIMIT: Duration = Duration::from_secs(30);
/// Result text of a call not started because the round ran out of time.
pub const ROUND_LIMIT_NOTE: &str = "error: not run; round time limit reached";
/// Result text of a call stopped because the turn was cancelled.
pub const CANCELLED_NOTE: &str = "error: cancelled";

/// Cancel flag of a turn; set when the turn future is dropped (Esc, /stop, switch, quit).
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Owns a [`Cancel`] and sets it on drop.
#[derive(Default)]
pub struct CancelOnDrop(Cancel);

impl CancelOnDrop {
    pub fn flag(&self) -> Cancel {
        self.0.clone()
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.set();
    }
}

/// Deadline plus cancel flag of one round.
#[derive(Clone)]
pub struct Budget {
    pub until: Instant,
    pub cancel: Cancel,
}

impl Budget {
    pub fn new(limit: Duration, cancel: Cancel) -> Self {
        Self {
            until: Instant::now() + limit,
            cancel,
        }
    }
    pub fn expired(&self) -> bool {
        Instant::now() >= self.until
    }
    /// The `error:` text to return when the work must stop now, if it must.
    pub fn stop_note(&self) -> Option<&'static str> {
        if self.cancel.is_set() {
            Some(CANCELLED_NOTE)
        } else if self.expired() {
            Some(ROUND_LIMIT_NOTE)
        } else {
            None
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Budget>> = const { RefCell::new(None) };
}

/// Run `body` with `budget` visible to tools on this thread.
pub fn within<T>(budget: &Budget, body: impl FnOnce() -> T) -> T {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            CURRENT.with(|current| *current.borrow_mut() = None);
        }
    }
    CURRENT.with(|current| *current.borrow_mut() = Some(budget.clone()));
    let _leave = Leave;
    body()
}

/// The budget of the call running on this thread, if a round set one.
pub fn current() -> Option<Budget> {
    CURRENT.with(|current| current.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_the_owner_sets_the_flag_and_stop_note_prefers_cancel() {
        let owner = CancelOnDrop::default();
        let flag = owner.flag();
        let budget = Budget::new(Duration::ZERO, flag.clone());
        assert_eq!(budget.stop_note(), Some(ROUND_LIMIT_NOTE));
        drop(owner);
        assert!(flag.is_set());
        assert_eq!(budget.stop_note(), Some(CANCELLED_NOTE));
    }

    #[test]
    fn the_budget_is_visible_only_inside_within() {
        let budget = Budget::new(Duration::from_secs(5), Cancel::default());
        assert!(current().is_none());
        within(&budget, || assert!(current().is_some()));
        assert!(current().is_none());
    }
}
