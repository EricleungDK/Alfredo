//! Monotonic UI observations. These do not claim server-side execution timing.
use std::time::Instant;

/// A coordinator observation is Alfredo client scheduling, not server capacity.
pub fn queue_summary(observation: &crate::inference_admission::Observation) -> String {
    use crate::inference_admission::Class;
    let class = match observation.class {
        Class::Foreground => "foreground",
        Class::Background => "background",
    };
    format!(
        "Shared Alfredo capacity · {class} · position {}/{} · {}/{} active",
        observation.position, observation.waiting, observation.active, observation.capacity
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    started: Instant,
    admitted: Option<Instant>,
    first: Option<Instant>,
    ended: Option<Instant>,
    /// Intervals spent waiting on the owner (an approval card). They are not model
    /// time, so every reported duration leaves them out. An open one has no end.
    pauses: Vec<(Instant, Option<Instant>)>,
}
impl Timing {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            admitted: None,
            first: None,
            ended: None,
            pauses: Vec::new(),
        }
    }
    /// The owner's turn starts: stop counting model time.
    pub fn pause(&mut self, now: Instant) {
        if self.ended.is_none() && !self.paused() {
            self.pauses.push((now, None));
        }
    }
    /// The owner answered: model time counts again.
    pub fn resume(&mut self, now: Instant) {
        if let Some((_, end)) = self.pauses.last_mut().filter(|(_, end)| end.is_none()) {
            *end = Some(now);
        }
    }
    pub fn paused(&self) -> bool {
        self.pauses.iter().any(|(_, end)| end.is_none())
    }
    /// Seconds from `from` to `to`, minus the owner-wait overlapping that span.
    fn span(&self, to: Instant, from: Instant) -> f64 {
        let mut total = to.saturating_duration_since(from);
        for (start, end) in &self.pauses {
            let end = end.or(self.ended).unwrap_or(to);
            let (start, end) = ((*start).max(from), end.min(to));
            total = total.saturating_sub(end.saturating_duration_since(start));
        }
        total.as_secs_f64()
    }
    pub fn admit(&mut self, now: Instant) {
        if self.ended.is_none() {
            self.admitted.get_or_insert(now);
        }
    }
    pub fn has_admission(&self) -> bool {
        self.admitted.is_some()
    }
    pub fn text(&mut self, now: Instant) {
        if self.ended.is_none() {
            self.first.get_or_insert(now);
        }
    }
    pub fn finish(&mut self, now: Instant) {
        self.ended.get_or_insert(now);
    }
    /// Short form for the transcript: elapsed seconds only.
    pub fn compact(&self, now: Instant) -> String {
        let end = self.ended.unwrap_or(now);
        format!("{:.1}s", self.span(end, self.started))
    }
    pub fn summary(&self, now: Instant) -> String {
        let end = self.ended.unwrap_or(now);
        let seconds = |to: Instant, from: Instant| self.span(to, from);
        let total = seconds(end, self.started);
        let Some(admitted) = self.admitted else {
            return format!(
                "Client · admission {} · total {total:.1}s",
                if self.ended.is_some() {
                    "not observed"
                } else {
                    "pending"
                }
            );
        };
        let queue = seconds(admitted, self.started);
        match self.first {
            Some(first) => format!("Client · queue {queue:.1}s · first text {:.1}s · stream {:.1}s · total {total:.1}s", seconds(first, admitted), seconds(end, first)),
            None => format!("Client · queue {queue:.1}s · {} {:.1}s · total {total:.1}s", if self.ended.is_some() { "no text after" } else { "waiting for text" }, seconds(end, admitted)),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn owner_wait_is_left_out_of_every_duration() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut timing = Timing::new(start);
        timing.admit(at(1));
        timing.pause(at(3));
        assert_eq!(timing.compact(at(30)), "3.0s", "open pause is excluded");
        timing.resume(at(33));
        assert_eq!(timing.compact(at(34)), "4.0s");
        timing.pause(at(35));
        timing.finish(at(40));
        assert_eq!(
            timing.compact(at(90)),
            "5.0s",
            "finish closes an open pause"
        );
        timing.resume(at(95));
        assert!(!timing.paused());
    }
    #[test]
    fn phases_use_distinct_intervals_and_terminal_observations_are_frozen() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut timing = Timing::new(start);
        assert_eq!(
            timing.summary(at(2)),
            "Client · admission pending · total 2.0s"
        );
        timing.admit(at(3));
        timing.admit(at(4));
        assert!(timing.summary(at(5)).contains("waiting for text 2.0s"));
        timing.text(at(7));
        timing.text(at(8));
        timing.finish(at(10));
        timing.finish(at(11));
        assert_eq!(
            timing.summary(at(30)),
            "Client · queue 3.0s · first text 4.0s · stream 3.0s · total 10.0s"
        );
        let mut cancelled = Timing::new(start);
        cancelled.finish(at(2));
        cancelled.admit(at(3));
        cancelled.text(at(4));
        assert_eq!(
            cancelled.summary(at(10)),
            "Client · admission not observed · total 2.0s"
        );
    }
}
