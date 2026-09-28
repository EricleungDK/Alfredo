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
