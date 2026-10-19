//! Optional server observations, not client latency or task authority.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metrics {
    pub total_duration: Option<u64>,
    pub load_duration: Option<u64>,
    pub prompt_eval_duration: Option<u64>,
    pub eval_duration: Option<u64>,
    pub prompt_eval_count: Option<u64>,
    pub eval_count: Option<u64>,
}
#[derive(Deserialize, Default)]
pub(crate) struct RawMetrics {
    total_duration: Option<serde_json::Value>,
    load_duration: Option<serde_json::Value>,
    prompt_eval_duration: Option<serde_json::Value>,
    eval_duration: Option<serde_json::Value>,
    prompt_eval_count: Option<serde_json::Value>,
    eval_count: Option<serde_json::Value>,
}
impl RawMetrics {
    pub fn bounded(self) -> Option<Metrics> {
        let number = |value: Option<serde_json::Value>, max| {
            value
                .and_then(|value| value.as_u64())
                .filter(|value| *value <= max)
        };
        let metrics = Metrics {
            total_duration: number(self.total_duration, 600_000_000_000),
            load_duration: number(self.load_duration, 600_000_000_000),
            prompt_eval_duration: number(self.prompt_eval_duration, 600_000_000_000),
            eval_duration: number(self.eval_duration, 600_000_000_000),
            prompt_eval_count: number(self.prompt_eval_count, 1_000_000),
            eval_count: number(self.eval_count, 1_000_000),
        };
        (metrics != Metrics::default()).then_some(metrics)
    }
}
impl Metrics {
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for (label, value) in [
            ("load", self.load_duration),
            ("prompt", self.prompt_eval_duration),
            ("generate", self.eval_duration),
            ("total", self.total_duration),
        ] {
            if let Some(ns) = value {
                parts.push(format!("{label} {:.2}s", ns as f64 / 1e9));
            }
        }
        if let (Some(count), Some(duration)) = (
            self.eval_count,
            self.eval_duration.filter(|value| *value > 0),
        ) {
            parts.push(format!(
                "{:.1} generated tokens/s",
                count as f64 / (duration as f64 / 1e9)
            ));
        }
        if let Some(count) = self.prompt_eval_count {
            parts.push(format!("{count} prompt tokens"));
        }
        format!("Server timing · {}", parts.join(" · "))
    }
}

/// Counts and model time worth keeping in a receipt: no per-phase timings, so a
/// plan receipt grows by well under a hundred bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_ns: Option<u64>,
}
impl Usage {
    pub fn from_metrics(metrics: &Metrics) -> Option<Self> {
        let usage = Self {
            prompt_tokens: metrics.prompt_eval_count,
            completion_tokens: metrics.eval_count,
            model_ns: metrics.total_duration,
        };
        (usage != Self::default() && usage.in_bounds()).then_some(usage)
    }
    /// Generous sanity limits (a billion tokens, 24 h); beyond them the numbers are dropped, not trusted.
    pub fn in_bounds(&self) -> bool {
        [self.prompt_tokens, self.completion_tokens]
            .into_iter()
            .flatten()
            .all(|tokens| tokens <= 1_000_000_000)
            && self.model_ns.is_none_or(|ns| ns <= 86_400_000_000_000)
    }
    /// Sum another phase in; a count the server never reported stays absent.
    pub fn add(&mut self, other: &Self) {
        let sum = |a: Option<u64>, b: Option<u64>| match (a, b) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
        };
        self.prompt_tokens = sum(self.prompt_tokens, other.prompt_tokens);
        self.completion_tokens = sum(self.completion_tokens, other.completion_tokens);
        self.model_ns = sum(self.model_ns, other.model_ns);
    }
    /// `tokens 12.3k in / 1.1k out · model 38 s`; absent when the server gave no token counts.
    pub fn line(&self) -> Option<String> {
        if self.prompt_tokens.is_none() && self.completion_tokens.is_none() {
            return None;
        }
        let count = |n: u64| match n {
            0..=999 => n.to_string(),
            _ => format!("{:.1}k", n as f64 / 1000.0),
        };
        let mut line = format!(
            "tokens {} in / {} out",
            count(self.prompt_tokens.unwrap_or(0)),
            count(self.completion_tokens.unwrap_or(0))
        );
        if let Some(ns) = self.model_ns {
            let seconds = ns as f64 / 1e9;
            if seconds < 1.0 {
                line.push_str(" · model <1 s");
            } else {
                line.push_str(&format!(" · model {seconds:.0} s"));
            }
        }
        Some(line)
    }
}
