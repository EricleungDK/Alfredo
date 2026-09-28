//! Opt-in diagnostic request identities. These observations never authorize work.
use crate::{
    inference_admission::Class, inference_runtime::Observation, metrics::Metrics,
    model::MAX_MESSAGES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

pub const MAX_REQUESTS: usize = 128;
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextProfile {
    #[default]
    Baseline,
    ContextCandidate,
}
impl ContextProfile {
    pub fn context(self, class: Class) -> Option<u32> {
        match (self, class) {
            (Self::Baseline, _) => None,
            (Self::ContextCandidate, Class::Foreground) => Some(8192),
            (Self::ContextCandidate, Class::Background) => Some(16384),
        }
    }
}

/// None means omitted from the HTTP payload, never an inferred server default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileIdentity {
    pub version: u32,
    pub context_profile: ContextProfile,
    pub endpoint_origin: String,
    pub model: String,
    pub class: Class,
    pub capacity: usize,
    pub connect_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub stream: bool,
    pub num_predict: u32,
    pub num_ctx: Option<u32>,
    pub temperature: Option<u8>,
    pub think: Option<bool>,
    pub format_sha256: Option<String>,
    pub keep_alive: Option<String>,
}
impl ProfileIdentity {
    pub fn validate(&self) -> Result<(), String> {
        let endpoint = reqwest::Url::parse(&self.endpoint_origin)
            .map_err(|_| "Invalid request profile origin")?;
        if self.version != 1
            || self.endpoint_origin.len() > 2048
            || endpoint.origin().ascii_serialization() != self.endpoint_origin
            || !matches!(endpoint.scheme(), "http" | "https")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || !valid_label(&self.model, 200)
            || !(1..=8).contains(&self.capacity)
            || self.connect_timeout_ms != 5000
            || self.total_timeout_ms != 600_000
            || self.idle_timeout_ms == 0
            || !self.stream
            || self.num_predict != 4096
            || self.num_ctx != self.context_profile.context(self.class)
            || self.keep_alive.is_some()
            || self
                .format_sha256
                .as_ref()
                .is_some_and(|value| !valid_digest(value))
            || match &self.format_sha256 {
                Some(_) => self.temperature != Some(0),
                None => self.temperature.is_some() || self.think.is_some(),
            }
        {
            return Err("Invalid or unsupported request profile identity".into());
        }
        Ok(())
    }
    pub fn sha256(&self) -> Result<String, String> {
        self.validate()?;
        canonical_digest(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageIdentity {
    pub role: String,
    pub content_bytes: usize,
    pub content_sha256: String,
    /// Hash of this exact serialized message object, including role and JSON escaping.
    pub wire_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestOutcome {
    InFlight,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestRecord {
    pub sequence: usize,
    pub session: usize,
    pub attempt: u64,
    pub profile: ProfileIdentity,
    pub profile_sha256: String,
    pub request_sha256: String,
    pub request_bytes: usize,
    pub messages: Vec<MessageIdentity>,
    /// Prefix convention: all messages except the final one, preserving exact order.
    pub prefix_messages: usize,
    /// SHA-256 of that actual serialized message array, without raw prompt retention.
    pub prefix_wire_sha256: String,
    /// Binds the serialized prefix to every profile field; not proof of a server cache hit.
    pub prefix_sha256: String,
    pub queue_ms: u64,
    pub first_content_ms: Option<u64>,
    /// Request start to terminal generation frame, before diagnostic inspection.
    pub generation_ms: Option<u64>,
    /// Instrumented provider lifetime, including the separately measured inspection.
    pub total_ms: Option<u64>,
    pub metrics: Option<Metrics>,
    pub runtime_after: Option<Observation>,
    pub runtime_error: Option<String>,
    pub runtime_probe_ms: Option<u64>,
    pub outcome: RequestOutcome,
}
impl RequestRecord {
    pub fn matches_binding(
        &self,
        profile: ContextProfile,
        origin: &str,
        model: &str,
        capacity: usize,
    ) -> bool {
        self.validate().is_ok()
            && self.profile.context_profile == profile
            && self.profile.endpoint_origin == origin
            && self.profile.model == model
            && self.profile.capacity == capacity
    }
    /// Validates retained relationships. Raw payload hashes cannot be reconstructed
    /// from content hashes; they are observed at dispatch, not an upstream attestation.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=MAX_REQUESTS).contains(&self.sequence)
            || !(1..=MAX_REQUEST_BYTES).contains(&self.request_bytes)
            || self.profile.sha256()? != self.profile_sha256
            || !valid_digest(&self.request_sha256)
            || !valid_digest(&self.prefix_wire_sha256)
            || self.messages.len() > MAX_MESSAGES
            || self.prefix_messages != self.messages.len().saturating_sub(1)
            || self.messages.iter().any(|message| {
                !matches!(
                    message.role.as_str(),
                    "system" | "user" | "assistant" | "tool"
                ) || message.content_bytes > MAX_REQUEST_BYTES
                    || !valid_digest(&message.content_sha256)
                    || !valid_digest(&message.wire_sha256)
            })
            || self
                .messages
                .iter()
                .map(|message| message.content_bytes)
                .sum::<usize>()
                > self.request_bytes
            || self.prefix_sha256
                != prefix_digest(
                    &self.profile_sha256,
                    &self.prefix_wire_sha256,
                    self.prefix_messages,
                )?
            || self.queue_ms > 600_000
            || self
                .first_content_ms
                .is_some_and(|value| value < self.queue_ms || value > 601_000)
            || self
                .total_ms
                .is_some_and(|value| value < self.queue_ms || value > 601_000)
            || self
                .generation_ms
                .is_some_and(|value| value < self.queue_ms || value > 601_000)
            || self
                .generation_ms
                .zip(self.total_ms)
                .is_some_and(|(generation, total)| generation > total)
            || self
                .first_content_ms
                .zip(self.generation_ms)
                .is_some_and(|(first, generation)| first > generation)
            || matches!(self.outcome, RequestOutcome::InFlight) != self.total_ms.is_none()
            || (self.outcome == RequestOutcome::Completed && self.generation_ms.is_none())
            || self
                .first_content_ms
                .zip(self.total_ms)
                .is_some_and(|(first, total)| first > total)
            || self
                .metrics
                .as_ref()
                .is_some_and(|metrics| !valid_metrics(metrics))
            || (self.runtime_after.is_some() && self.runtime_error.is_some())
            || (self.outcome != RequestOutcome::InFlight
                && self.runtime_after.is_none()
                && self.runtime_error.is_none())
            || (self.runtime_after.is_some()
                && (self.generation_ms.is_none() || self.runtime_probe_ms.is_none()))
            || self
                .runtime_error
                .as_ref()
                .is_some_and(|error| !valid_label(error, 2048))
            || self.runtime_probe_ms.is_some_and(|value| value > 601_000)
            || self
                .runtime_probe_ms
                .zip(self.total_ms)
                .is_some_and(|(probe, total)| probe > total)
        {
            return Err("Invalid recorded inference request".into());
        }
        if let Some(runtime) = &self.runtime_after {
            runtime.validate()?;
            if runtime.endpoint_origin != self.profile.endpoint_origin
                || runtime.selected_model != self.profile.model
            {
                return Err("Recorded runtime belongs to a different request".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct RequestRecorder {
    inner: Arc<Mutex<Records>>,
}
/// Read-only live observations for fixture overlap evidence, never request authority.
#[derive(Clone)]
pub struct RequestObserver {
    recorder: RequestRecorder,
}
impl RequestObserver {
    pub fn active_generations(&self, class: Class) -> Result<Vec<usize>, String> {
        let records = self
            .recorder
            .inner
            .lock()
            .map_err(|_| "Request recorder is unavailable")?;
        Ok(records
            .records
            .iter()
            .filter(|record| {
                record.profile.class == class
                    && record.outcome == RequestOutcome::InFlight
                    && record.generation_ms.is_none()
            })
            .map(|record| record.sequence)
            .collect())
    }
    pub fn generation_active(&self, sequence: usize, class: Class) -> Result<bool, String> {
        Ok(self.active_generations(class)?.contains(&sequence))
    }
}
struct Records {
    limit: usize,
    records: Vec<RequestRecord>,
}
impl Default for RequestRecorder {
    fn default() -> Self {
        Self::new()
    }
}
impl RequestRecorder {
    pub fn observer(&self) -> RequestObserver {
        RequestObserver {
            recorder: self.clone(),
        }
    }
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Records {
                limit: MAX_REQUESTS,
                records: Vec::new(),
            })),
        }
    }
    pub fn with_limit(limit: usize) -> Result<Self, String> {
        if !(1..=MAX_REQUESTS).contains(&limit) {
            return Err("Request recorder limit must be between 1 and 128".into());
        }
        Ok(Self {
            inner: Arc::new(Mutex::new(Records {
                limit,
                records: Vec::new(),
            })),
        })
    }
    pub fn snapshot(&self) -> Result<Vec<RequestRecord>, String> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| "Request recorder is unavailable")?
            .records
            .clone())
    }
    pub(crate) fn begin(
        &self,
        identity: (usize, u64),
        profile: ProfileIdentity,
        payload: &[u8],
        started: Instant,
        queue_ms: u64,
    ) -> Result<Recording, String> {
        if payload.is_empty() || payload.len() > MAX_REQUEST_BYTES {
            return Err("Recorded inference payload exceeds 4 MiB".into());
        }
        // Parse the serialized bytes that will be sent, not a second independently built request.
        let body: serde_json::Value =
            serde_json::from_slice(payload).map_err(|_| "Invalid recorded request payload")?;
        validate_payload(&profile, &body)?;
        let messages = body
            .get("messages")
            .and_then(serde_json::Value::as_array)
            .ok_or("Recorded request has no messages")?;
        if messages.len() > MAX_MESSAGES {
            return Err("Recorded request has too many messages".into());
        }
        let mut identities = Vec::with_capacity(messages.len());
        for message in messages {
            let role = message
                .get("role")
                .and_then(serde_json::Value::as_str)
                .ok_or("Recorded message has no role")?;
            let content = message
                .get("content")
                .and_then(serde_json::Value::as_str)
                .ok_or("Recorded message has no content")?;
            identities.push(MessageIdentity {
                role: role.into(),
                content_bytes: content.len(),
                content_sha256: digest(content.as_bytes()),
                wire_sha256: canonical_digest(message)?,
            });
        }
        let prefix_messages = messages.len().saturating_sub(1);
        let prefix_wire_sha256 = canonical_digest(&messages[..prefix_messages])?;
        let profile_sha256 = profile.sha256()?;
        let mut records = self
            .inner
            .lock()
            .map_err(|_| "Request recorder is unavailable")?;
        if records.records.len() >= records.limit {
            return Err("Qualification request limit reached before HTTP dispatch".into());
        }
        let index = records.records.len();
        let record = RequestRecord {
            sequence: index + 1,
            session: identity.0,
            attempt: identity.1,
            profile,
            prefix_sha256: prefix_digest(&profile_sha256, &prefix_wire_sha256, prefix_messages)?,
            profile_sha256,
            request_sha256: digest(payload),
            request_bytes: payload.len(),
            messages: identities,
            prefix_messages,
            prefix_wire_sha256,
            queue_ms,
            first_content_ms: None,
            generation_ms: None,
            total_ms: None,
            metrics: None,
            runtime_after: None,
            runtime_error: None,
            runtime_probe_ms: None,
            outcome: RequestOutcome::InFlight,
        };
        record.validate()?;
        records.records.push(record);
        Ok(Recording {
            recorder: self.clone(),
            index,
            started,
            probe_started: None,
            finished: false,
        })
    }
}

pub(crate) struct Recording {
    recorder: RequestRecorder,
    index: usize,
    started: Instant,
    probe_started: Option<Instant>,
    finished: bool,
}
impl Recording {
    pub(crate) fn content(&mut self) {
        let elapsed = elapsed_ms(self.started);
        self.update(|record| {
            record.first_content_ms.get_or_insert(elapsed);
        });
    }
    pub(crate) fn metrics(&mut self, metrics: Metrics) {
        self.update(|record| record.metrics = Some(metrics));
    }
    pub(crate) fn generated(&mut self) {
        let elapsed = elapsed_ms(self.started);
        self.update(|record| {
            record.generation_ms.get_or_insert(elapsed);
        });
    }
    pub(crate) fn begin_runtime(&mut self) {
        self.probe_started = Some(Instant::now());
    }
    pub(crate) fn runtime(&mut self, result: Result<Observation, String>) {
        let elapsed = self.probe_started.map(elapsed_ms);
        self.update(|record| {
            record.runtime_probe_ms = elapsed;
            match result {
                Ok(runtime) => record.runtime_after = Some(runtime),
                Err(error) => {
                    let safe: String = error
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(500)
                        .collect();
                    record.runtime_error = Some(if safe.trim().is_empty() {
                        "Runtime inspection failed".into()
                    } else {
                        safe
                    });
                }
            }
        });
    }
    pub(crate) fn finish(&mut self, outcome: RequestOutcome) {
        if self.finished {
            return;
        }
        let elapsed = elapsed_ms(self.started);
        let probe = self.probe_started.map(elapsed_ms);
        self.update(|record| {
            record.total_ms = Some(elapsed);
            record.outcome = outcome;
            if record.runtime_after.is_none() && record.runtime_error.is_none() {
                record.runtime_error =
                    Some("Request ended before runtime inspection completed".into());
                record.runtime_probe_ms = probe;
            }
        });
        self.finished = true;
    }
    fn update(&self, update: impl FnOnce(&mut RequestRecord)) {
        if let Ok(mut records) = self.recorder.inner.lock() {
            if let Some(record) = records.records.get_mut(self.index) {
                update(record);
            }
        }
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        self.finish(RequestOutcome::Interrupted);
    }
}
pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}
fn valid_label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn canonical_digest<T: Serialize + ?Sized>(value: &T) -> Result<String, String> {
    // Converting through Value gives stable sorted object keys even for structs/maps.
    let value =
        serde_json::to_value(value).map_err(|_| "Could not serialize inference identity")?;
    let bytes = serde_json::to_vec(&value).map_err(|_| "Could not serialize inference identity")?;
    Ok(digest(&bytes))
}
fn prefix_digest(profile: &str, wire: &str, count: usize) -> Result<String, String> {
    canonical_digest(
        &serde_json::json!({"profile_sha256":profile,"prefix_wire_sha256":wire,"prefix_messages":count}),
    )
}
fn valid_metrics(metrics: &Metrics) -> bool {
    [
        metrics.total_duration,
        metrics.load_duration,
        metrics.prompt_eval_duration,
        metrics.eval_duration,
    ]
    .into_iter()
    .flatten()
    .all(|value| value <= 600_000_000_000)
        && [metrics.prompt_eval_count, metrics.eval_count]
            .into_iter()
            .flatten()
            .all(|value| value <= 1_000_000)
}

fn validate_payload(profile: &ProfileIdentity, body: &serde_json::Value) -> Result<(), String> {
    let options = body
        .get("options")
        .and_then(serde_json::Value::as_object)
        .ok_or("Recorded payload has no options")?;
    let format = body.get("format").map(canonical_digest).transpose()?;
    let context = options.get("num_ctx").map(serde_json::Value::as_u64);
    let temperature = options.get("temperature").map(serde_json::Value::as_u64);
    let think = body.get("think").map(serde_json::Value::as_bool);
    if body.as_object().map(serde_json::Map::len)
        != Some(4 + usize::from(format.is_some()) + usize::from(think.is_some()))
        || options.len() != 1 + usize::from(context.is_some()) + usize::from(temperature.is_some())
        || body.get("model").and_then(serde_json::Value::as_str) != Some(profile.model.as_str())
        || body.get("stream").and_then(serde_json::Value::as_bool) != Some(profile.stream)
        || options
            .get("num_predict")
            .and_then(serde_json::Value::as_u64)
            != Some(u64::from(profile.num_predict))
        || context != profile.num_ctx.map(|value| Some(u64::from(value)))
        || temperature != profile.temperature.map(|value| Some(u64::from(value)))
        || think != profile.think.map(Some)
        || format != profile.format_sha256
    {
        return Err("Recorded profile differs from actual request payload".into());
    }
    Ok(())
}
