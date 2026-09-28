use crate::{
    inference_admission::{Class, Coordinator},
    inference_profile::{
        self, ContextProfile, ProfileIdentity, Recording, RequestOutcome, RequestRecorder,
    },
    model::{Event, Message, Retry, Update, MAX_TEXT},
};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;

const MAX_FRAME: usize = 64 * 1024;
const MAX_PREDICT: u32 = 4096;
const MAX_CONNECT_RETRIES: u32 = 10;
const PRELOAD_DEADLINE: Duration = Duration::from_secs(300);
const HEALTH_DEADLINE: Duration = Duration::from_secs(2);

/// Validate an Ollama `keep_alive` value. `default` omits the field so the
/// server default applies; integers are seconds (negative keeps loaded);
/// otherwise a Go duration such as `30m` or `1h30m`.
pub fn parse_keep_alive(value: &str) -> Result<Option<String>, String> {
    if value == "default" {
        return Ok(None);
    }
    let invalid = || {
        format!("Invalid keep-alive {value:?}; use a duration like 30m or 1h30m, seconds like 300, -1 to keep loaded, or default")
    };
    if value.is_empty() || value.len() > 32 {
        return Err(invalid());
    }
    let body = value.strip_prefix('-').unwrap_or(value);
    if !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(Some(value.into()));
    }
    let mut rest = body;
    if rest.is_empty() {
        return Err(invalid());
    }
    while !rest.is_empty() {
        let split = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .ok_or_else(invalid)?;
        let number = &rest[..split];
        if number.is_empty() || number.parse::<f64>().is_err() {
            return Err(invalid());
        }
        rest = &rest[split..];
        let unit = ["ns", "us", "µs", "ms", "h", "m", "s"]
            .into_iter()
            .find(|unit| rest.starts_with(unit))
            .ok_or_else(invalid)?;
        rest = &rest[unit.len()..];
    }
    Ok(Some(value.into()))
}

/// Integers are sent as JSON numbers (seconds); durations as strings.
fn keep_alive_json(value: &str) -> serde_json::Value {
    value
        .parse::<i64>()
        .map_or_else(|_| value.into(), serde_json::Value::from)
}

/// Pre-content failures carry no response and no side effects, so they are
/// safe to retry; anything after content keeps the manual-retry contract.
enum Failure {
    BeforeContent(String),
    Final(String),
}
impl From<String> for Failure {
    fn from(error: String) -> Self {
        Self::Final(error)
    }
}
impl From<&str> for Failure {
    fn from(error: &str) -> Self {
        Self::Final(error.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestedThinking {
    Auto,
    On,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub thinking: RequestedThinking,
    pub num_predict: u32,
    pub temperature: u8,
}
impl Generation {
    pub fn valid(&self) -> bool {
        (1..=131_072).contains(&self.num_predict) && self.temperature <= 2
    }
    pub fn summary(&self) -> String {
        let thinking = match self.thinking {
            RequestedThinking::Auto => "auto",
            RequestedThinking::On => "on",
            RequestedThinking::Off => "off",
        };
        format!(
            "Requested generation: thinking {thinking} · token limit {} · temperature {}",
            self.num_predict, self.temperature
        )
    }
}

#[derive(Clone)]
pub struct Ollama {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    idle_timeout: Duration,
    format: Option<serde_json::Value>,
    structured_thinking: Option<bool>,
    admission: Coordinator,
    priority: Class,
    capacity: usize,
    context_profile: ContextProfile,
    recorder: Option<RequestRecorder>,
    keep_alive: Option<String>,
    connect_retries: u32,
    retry_backoff: Duration,
}

#[derive(Default)]
struct Trace {
    started: Option<Instant>,
    recording: Option<Recording>,
}

#[derive(Deserialize)]
struct Frame {
    #[serde(flatten)]
    metrics: crate::metrics::RawMetrics,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    done_reason: Option<String>,
    #[serde(default)]
    message: Option<Content>,
}

#[derive(Deserialize)]
struct Content {
    content: Option<String>,
    thinking: Option<String>,
}

#[derive(Default)]
struct FrameProgress {
    bytes: usize,
    thinking_observed: bool,
}
impl FrameProgress {
    fn failure(&self, error: &str) -> Failure {
        if self.bytes == 0 && !self.thinking_observed {
            Failure::BeforeContent(error.into())
        } else {
            Failure::Final(error.into())
        }
    }
}

impl Ollama {
    pub fn new(base: &str, idle_timeout: Duration) -> Result<Self, String> {
        let mut endpoint = reqwest::Url::parse(base).map_err(|_| "Invalid Ollama URL")?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.path() != "/"
        {
            return Err(
                "Ollama URL must be an HTTP(S) origin without credentials or a path".into(),
            );
        }
        endpoint.set_path("/api/chat");
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| "Could not initialize HTTP client")?;
        let admission = Coordinator::new(endpoint.origin().ascii_serialization(), 2)?;
        Ok(Self {
            client,
            endpoint,
            idle_timeout,
            format: None,
            structured_thinking: Some(false),
            admission,
            priority: Class::Foreground,
            capacity: 2,
            context_profile: ContextProfile::Baseline,
            recorder: None,
            keep_alive: None,
            connect_retries: 0,
            retry_backoff: Duration::from_secs(1),
        })
    }

    /// None omits `keep_alive` (server default). Use `parse_keep_alive` first.
    pub fn with_keep_alive(mut self, keep_alive: Option<String>) -> Self {
        self.keep_alive = keep_alive;
        self
    }

    /// Automatic retries before any response content; 0 disables.
    pub fn with_connect_retries(mut self, retries: u32) -> Result<Self, String> {
        if retries > MAX_CONNECT_RETRIES {
            return Err(format!(
                "Connect retries must be between 0 and {MAX_CONNECT_RETRIES}"
            ));
        }
        self.connect_retries = retries;
        Ok(self)
    }

    /// First backoff delay; each later retry doubles it.
    pub fn with_retry_backoff(mut self, first: Duration) -> Self {
        self.retry_backoff = first;
        self
    }

    /// Load `model` without generating. Bypasses inference admission like
    /// model discovery; callers treat failure as status only.
    pub async fn preload(&self, model: &str) -> Result<(), String> {
        let mut endpoint = self.endpoint.clone();
        endpoint.set_path("/api/generate");
        let mut body = serde_json::json!({"model": model, "prompt": "", "stream": false});
        if let Some(keep_alive) = &self.keep_alive {
            body["keep_alive"] = keep_alive_json(keep_alive);
        }
        tokio::time::timeout(PRELOAD_DEADLINE, async {
            let mut response = self
                .client
                .post(endpoint)
                .json(&body)
                .send()
                .await
                .map_err(|_| "Cannot reach Ollama to preload")?;
            if !response.status().is_success() {
                return Err(format!(
                    "Preload returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            let mut bytes = 0;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Preload connection lost")?
            {
                bytes += chunk.len();
                if bytes > MAX_FRAME {
                    return Err("Preload response exceeds 64 KiB".into());
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| "Preload exceeded its five-minute deadline".to_string())?
    }

    /// Names of resident models (GET /api/ps); a short health probe that
    /// bypasses inference admission.
    pub async fn running_models(&self) -> Result<Vec<String>, String> {
        tokio::time::timeout(HEALTH_DEADLINE, async {
            let mut endpoint = self.endpoint.clone();
            endpoint.set_path("/api/ps");
            let mut response = self
                .client
                .get(endpoint)
                .send()
                .await
                .map_err(|_| "Cannot reach Ollama")?;
            if !response.status().is_success() {
                return Err(format!(
                    "Ollama health returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Ollama health connection lost")?
            {
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err("Ollama health response exceeds 1 MiB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            #[derive(Deserialize)]
            struct Running {
                #[serde(default)]
                models: Option<Vec<Entry>>,
            }
            #[derive(Deserialize)]
            struct Entry {
                #[serde(default)]
                name: Option<String>,
                #[serde(default)]
                model: Option<String>,
            }
            let running: Running =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid Ollama health response")?;
            let mut names: Vec<String> = running
                .models
                .unwrap_or_default()
                .into_iter()
                .take(256)
                .flat_map(|entry| [entry.name, entry.model])
                .flatten()
                .filter(|name| name.len() <= 200 && !name.chars().any(char::is_control))
                .collect();
            names.sort();
            names.dedup();
            Ok(names)
        })
        .await
        .map_err(|_| "Ollama health probe timed out".to_string())?
    }

    /// Configure before cloning the provider into conversations and workers.
    pub fn with_parallelism(mut self, capacity: usize) -> Result<Self, String> {
        if !(1..=8).contains(&capacity) {
            return Err("Model parallelism must be between 1 and 8".into());
        }
        self.admission = Coordinator::new(self.endpoint.origin().ascii_serialization(), capacity)?;
        self.capacity = capacity;
        Ok(self)
    }

    /// Explicit experiments only; ordinary requests retain omitted context settings.
    pub fn with_context_profile(mut self, profile: ContextProfile) -> Self {
        self.context_profile = profile;
        self
    }

    pub fn with_request_recorder(mut self, recorder: RequestRecorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    pub fn request_observer(&self) -> Option<inference_profile::RequestObserver> {
        self.recorder.as_ref().map(RequestRecorder::observer)
    }

    pub fn endpoint_origin(&self) -> String {
        self.endpoint.origin().ascii_serialization()
    }

    pub(crate) fn inspection_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Request role is explicit: JSON formatting does not determine scheduling priority.
    pub fn with_priority(mut self, priority: Class) -> Self {
        self.priority = priority;
        self
    }

    /// Only affects schema-constrained planner/worker calls; None uses server defaults.
    pub fn with_structured_thinking(mut self, thinking: Option<bool>) -> Self {
        self.structured_thinking = thinking;
        self
    }

    pub fn structured_generation(&self) -> Generation {
        Generation {
            thinking: match self.structured_thinking {
                None => RequestedThinking::Auto,
                Some(true) => RequestedThinking::On,
                Some(false) => RequestedThinking::Off,
            },
            num_predict: MAX_PREDICT,
            temperature: 0,
        }
    }

    pub async fn models(&self) -> Result<Vec<String>, String> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut endpoint = self.endpoint.clone();
            endpoint.set_path("/api/tags");
            let mut response = self
                .client
                .get(endpoint)
                .send()
                .await
                .map_err(|_| "Cannot reach Ollama for model discovery")?;
            if !response.status().is_success() {
                return Err(format!(
                    "Model discovery returned HTTP {}",
                    response.status().as_u16()
                ));
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| "Model discovery connection lost")?
            {
                if bytes.len() + chunk.len() > 1024 * 1024 {
                    return Err("Model catalog exceeds 1 MiB".into());
                }
                bytes.extend_from_slice(&chunk);
            }
            #[derive(Deserialize)]
            struct Catalog {
                models: Vec<Entry>,
            }
            #[derive(Deserialize)]
            struct Entry {
                name: String,
            }
            let catalog: Catalog =
                serde_json::from_slice(&bytes).map_err(|_| "Invalid Ollama model catalog")?;
            if catalog.models.len() > 256 {
                return Err("Model catalog exceeds 256 entries".into());
            }
            let mut names = Vec::new();
            for entry in catalog.models {
                if entry.name.trim().is_empty()
                    || entry.name.len() > 200
                    || entry.name.chars().any(char::is_control)
                {
                    return Err("Model catalog contains an invalid name".into());
                }
                names.push(entry.name);
            }
            names.sort();
            names.dedup();
            Ok(names)
        })
        .await
        .map_err(|_| "Model discovery exceeded its ten-second deadline".to_string())?
    }

    pub fn with_json_schema(mut self, schema: serde_json::Value) -> Self {
        self.format = Some(schema);
        self
    }

    pub async fn chat(
        &self,
        session: usize,
        attempt: u64,
        model: String,
        messages: Vec<Message>,
        sender: Sender<Event>,
    ) {
        self.chat_with_admission(session, attempt, model, messages, sender, || async {
            Ok(())
        })
        .await;
    }

    /// Revalidate captured inputs after shared capacity is acquired. Rejection
    /// produces a normal Failed event before any HTTP request is sent. The
    /// check runs again on every automatic reconnection.
    pub async fn chat_with_admission<Check, Checked>(
        &self,
        session: usize,
        attempt: u64,
        model: String,
        messages: Vec<Message>,
        sender: Sender<Event>,
        mut check: Check,
    ) where
        Check: FnMut() -> Checked,
        Checked: std::future::Future<Output = Result<(), String>>,
    {
        let mut trace = Trace {
            started: self.recorder.as_ref().map(|_| Instant::now()),
            recording: None,
        };
        // Recorded qualification requests measure one exact wire request.
        let limit = if self.recorder.is_some() {
            0
        } else {
            self.connect_retries
        };
        let result = tokio::time::timeout(Duration::from_secs(600), async {
            let mut retry = 0;
            loop {
                let outcome = self
                    .stream(
                        (session, attempt),
                        model.clone(),
                        messages.clone(),
                        &sender,
                        &mut check,
                        &mut trace,
                    )
                    .await;
                match outcome {
                    Ok(()) => return Ok(()),
                    Err(Failure::BeforeContent(reason)) if retry < limit => {
                        retry += 1;
                        // Shared capacity is released while waiting.
                        let delay = self.retry_backoff.saturating_mul(1 << (retry - 1));
                        sender
                            .send(Event {
                                session,
                                attempt,
                                update: Update::Retrying(Retry {
                                    retry,
                                    limit,
                                    delay,
                                    reason,
                                }),
                            })
                            .await
                            .map_err(|_| "Conversation closed during reconnection".to_string())?;
                        tokio::time::sleep(delay).await;
                    }
                    Err(Failure::BeforeContent(error) | Failure::Final(error)) => {
                        return Err(error)
                    }
                }
            }
        })
        .await;
        if let Some(recording) = &mut trace.recording {
            recording.finish(if matches!(&result, Ok(Ok(()))) {
                RequestOutcome::Completed
            } else {
                RequestOutcome::Failed
            });
        }
        let failure = match result {
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => "Request exceeded the ten-minute deadline; partial reply retained".into(),
        };
        let _ = sender
            .send(Event {
                session,
                attempt,
                update: Update::Failed(failure),
            })
            .await;
    }

    async fn stream<Check, Checked>(
        &self,
        identity: (usize, u64),
        model: String,
        messages: Vec<Message>,
        sender: &Sender<Event>,
        check: &mut Check,
        trace: &mut Trace,
    ) -> Result<(), Failure>
    where
        Check: FnMut() -> Checked,
        Checked: std::future::Future<Output = Result<(), String>>,
    {
        let (session, attempt) = identity;
        // Independently constructed providers and other terminals coordinate by
        // normalized endpoint. Dropping Queue removes eligibility before HTTP;
        // Permit retains shared capacity until this stream exits.
        let mut queue = self.admission.queue(self.priority);
        let mut queued = false;
        let mut observed = None;
        let _permit = loop {
            if let Some(permit) = queue.poll()? {
                break permit;
            }
            if !queued {
                sender
                    .send(Event {
                        session,
                        attempt,
                        update: Update::Queued,
                    })
                    .await
                    .map_err(|_| "Conversation closed while waiting for shared model capacity")?;
                queued = true;
            }
            let current = queue.observation();
            if current != observed {
                if let Some(observation) = current {
                    sender
                        .send(Event {
                            session,
                            attempt,
                            update: Update::QueueProgress(observation),
                        })
                        .await
                        .map_err(|_| {
                            "Conversation closed while waiting for shared model capacity"
                        })?;
                }
                observed = current;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let queue_ms = trace
            .started
            .map(inference_profile::elapsed_ms)
            .unwrap_or(0);
        check().await?;
        sender
            .send(Event {
                session,
                attempt,
                update: Update::Admitted,
            })
            .await
            .map_err(|_| "Conversation closed before model dispatch")?;
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
            "options": { "num_predict": MAX_PREDICT }
        });
        if let Some(format) = &self.format {
            body["format"] = format.clone();
            if let Some(thinking) = self.structured_thinking {
                body["think"] = thinking.into();
            }
            body["options"]["temperature"] = self.structured_generation().temperature.into();
        }
        if let Some(context) = self.context_profile.context(self.priority) {
            body["options"]["num_ctx"] = context.into();
        }
        if let Some(keep_alive) = &self.keep_alive {
            body["keep_alive"] = keep_alive_json(keep_alive);
        }
        // The recorder and HTTP body share these exact bytes. No second serialization
        // can turn requested settings into a different claim about the wire request.
        let payload = serde_json::to_vec(&body).map_err(|_| "Could not encode Ollama request")?;
        if let (Some(recorder), Some(started)) = (&self.recorder, trace.started) {
            let profile = ProfileIdentity {
                version: 1,
                context_profile: self.context_profile,
                endpoint_origin: self.endpoint_origin(),
                model: body["model"]
                    .as_str()
                    .ok_or("Invalid request model")?
                    .into(),
                class: self.priority,
                capacity: self.capacity,
                connect_timeout_ms: 5000,
                total_timeout_ms: 600_000,
                idle_timeout_ms: u64::try_from(self.idle_timeout.as_millis())
                    .map_err(|_| "Recorded idle deadline is too large")?,
                stream: true,
                num_predict: MAX_PREDICT,
                num_ctx: self.context_profile.context(self.priority),
                temperature: self.format.as_ref().map(|_| 0),
                think: self.format.as_ref().and(self.structured_thinking),
                format_sha256: self
                    .format
                    .as_ref()
                    .map(inference_profile::canonical_digest)
                    .transpose()?,
                keep_alive: self.keep_alive.clone(),
            };
            trace.recording = Some(recorder.begin(identity, profile, &payload, started, queue_ms)?);
        }
        let request = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload);
        let mut response = match tokio::time::timeout(self.idle_timeout, request.send()).await {
            Err(_) => {
                return Err(Failure::BeforeContent(
                    "Ollama did not respond before the loading deadline".into(),
                ))
            }
            Ok(Err(_)) => {
                return Err(Failure::BeforeContent(
                    "Cannot reach Ollama; check the server and endpoint, then retry".into(),
                ))
            }
            Ok(Ok(response)) => response,
        };
        if !response.status().is_success() {
            let error = format!(
                "Ollama returned HTTP {}; check the model and server",
                response.status().as_u16()
            );
            return Err(if response.status().is_server_error() {
                Failure::BeforeContent(error)
            } else {
                Failure::Final(error)
            });
        }
        let mut pending = Vec::new();
        let mut progress = FrameProgress::default();
        loop {
            let chunk = match tokio::time::timeout(self.idle_timeout, response.chunk()).await {
                Err(_) => return Err("Ollama stream stalled; partial reply retained".into()),
                Ok(Err(_)) => {
                    return Err(progress.failure("Ollama connection lost; partial reply retained"))
                }
                Ok(Ok(chunk)) => chunk,
            };
            let Some(chunk) = chunk else {
                if !pending.is_empty()
                    && self
                        .frame(&pending, session, attempt, sender, &mut progress, trace)
                        .await?
                {
                    return Ok(self
                        .complete_recorded(identity, &body, sender, trace)
                        .await?);
                }
                return Err(progress
                    .failure("Ollama disconnected before completion; partial reply retained"));
            };
            // Bound incomplete frames even when a server never supplies a newline.
            for byte in chunk {
                if byte == b'\n' {
                    if self
                        .frame(&pending, session, attempt, sender, &mut progress, trace)
                        .await?
                    {
                        return Ok(self
                            .complete_recorded(identity, &body, sender, trace)
                            .await?);
                    }
                    pending.clear();
                } else {
                    if pending.len() == MAX_FRAME {
                        return Err("Ollama stream frame exceeded 64 KiB".into());
                    }
                    pending.push(byte);
                }
            }
        }
    }

    async fn complete_recorded(
        &self,
        identity: (usize, u64),
        body: &serde_json::Value,
        sender: &Sender<Event>,
        trace: &mut Trace,
    ) -> Result<(), String> {
        let Some(recording) = &mut trace.recording else {
            return Ok(());
        };
        // Still inside stream(): its shared permit survives the inspection. A
        // worker sees Done only afterward, so its provider shutdown cannot cut
        // off the successful request's runtime proof. Inspection never queues.
        recording.begin_runtime();
        let model = body["model"]
            .as_str()
            .ok_or("Invalid recorded request model")?;
        recording.runtime(crate::inference_runtime::inspect(self, model).await);
        // Completion describes the fully observed generation, independently of
        // whether its caller receives Done or immediately drops this future.
        recording.finish(RequestOutcome::Completed);
        sender
            .send(Event {
                session: identity.0,
                attempt: identity.1,
                update: Update::Done,
            })
            .await
            .map_err(|_| "Terminal closed")?;
        Ok(())
    }

    async fn frame(
        &self,
        bytes: &[u8],
        session: usize,
        attempt: u64,
        sender: &Sender<Event>,
        progress: &mut FrameProgress,
        trace: &mut Trace,
    ) -> Result<bool, String> {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(false);
        }
        let frame: Frame =
            serde_json::from_slice(bytes).map_err(|_| "Malformed Ollama stream frame")?;
        if let Some(error) = frame.error {
            let safe: String = error
                .chars()
                .filter(|c| !c.is_control())
                .take(300)
                .collect();
            return Err(format!("Ollama: {safe}"));
        }
        if frame.message.is_none() && !frame.done {
            return Err("Ollama frame has neither message nor completion".into());
        }
        if let Some(message) = frame.message {
            if message.content.is_none() && message.thinking.is_none() {
                return Err("Ollama message has neither content nor thinking".into());
            }
            let content = message.content.unwrap_or_default();
            let thinking = message.thinking.unwrap_or_default();
            progress.bytes += content.len() + thinking.len();
            if progress.bytes > MAX_TEXT {
                return Err("Ollama output exceeded 128 KiB".into());
            }
            // Observe generation clocks before a slow event consumer can add
            // presentation backpressure to the final-frame timestamp.
            if let Some(recording) = &mut trace.recording {
                if !content.is_empty() {
                    recording.content();
                }
                if frame.done {
                    recording.generated();
                }
            }
            if !thinking.is_empty() && !progress.thinking_observed {
                progress.thinking_observed = true;
                sender
                    .send(Event {
                        session,
                        attempt,
                        update: Update::Thinking,
                    })
                    .await
                    .map_err(|_| "Terminal closed")?;
            }
            if !content.is_empty() {
                sender
                    .send(Event {
                        session,
                        attempt,
                        update: Update::Token(content),
                    })
                    .await
                    .map_err(|_| "Terminal closed")?;
            }
        }
        if frame.done {
            if let Some(recording) = &mut trace.recording {
                recording.generated();
            }
            if let Some(metrics) = frame.metrics.bounded() {
                if let Some(recording) = &mut trace.recording {
                    recording.metrics(metrics.clone());
                }
                sender
                    .send(Event {
                        session,
                        attempt,
                        update: Update::Metrics(metrics),
                    })
                    .await
                    .map_err(|_| "Terminal closed")?;
            }
            if frame.done_reason.as_deref() == Some("length") {
                return Err("Ollama reached its generation limit; partial reply retained. Shorten the request or choose another model before retrying".into());
            }
            if trace.recording.is_none() {
                sender
                    .send(Event {
                        session,
                        attempt,
                        update: Update::Done,
                    })
                    .await
                    .map_err(|_| "Terminal closed")?;
            }
        }
        Ok(frame.done)
    }
}
