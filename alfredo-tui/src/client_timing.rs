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
}
impl Timing {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            admitted: None,
            first: None,
            ended: None,
        }
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
        format!(
            "{:.1}s",
            end.saturating_duration_since(self.started).as_secs_f64()
        )
    }
    pub fn summary(&self, now: Instant) -> String {
        let end = self.ended.unwrap_or(now);
        let seconds = |to: Instant, from: Instant| to.saturating_duration_since(from).as_secs_f64();
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
