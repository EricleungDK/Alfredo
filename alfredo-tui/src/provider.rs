use crate::{
    inference_admission::{Class, Coordinator},
    inference_profile::{
        self, ContextProfile, ProfileIdentity, Recording, RequestOutcome, RequestRecorder,
    },
    model::{Event, Message, Update, MAX_TEXT},
};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;

const MAX_FRAME: usize = 64 * 1024;
const MAX_PREDICT: u32 = 4096;

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
        })
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
    /// produces a normal Failed event before any HTTP request is sent.
    pub async fn chat_with_admission<Check, Checked>(
        &self,
        session: usize,
        attempt: u64,
        model: String,
        messages: Vec<Message>,
        sender: Sender<Event>,
        check: Check,
    ) where
        Check: FnOnce() -> Checked,
        Checked: std::future::Future<Output = Result<(), String>>,
    {
        let mut trace = Trace {
            started: self.recorder.as_ref().map(|_| Instant::now()),
            recording: None,
        };
        let result = tokio::time::timeout(
            Duration::from_secs(600),
            self.stream(
                (session, attempt),
                model,
                messages,
                &sender,
                check,
                &mut trace,
            ),
        )
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
        check: Check,
        trace: &mut Trace,
    ) -> Result<(), String>
    where
        Check: FnOnce() -> Checked,
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
                keep_alive: None,
            };
            trace.recording = Some(recorder.begin(identity, profile, &payload, started, queue_ms)?);
        }
        let request = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(payload);
        let mut response = tokio::time::timeout(self.idle_timeout, request.send())
            .await
            .map_err(|_| "Ollama did not respond before the loading deadline")?
            .map_err(|_| "Cannot reach Ollama; check the server and endpoint, then retry")?;
        if !response.status().is_success() {
            return Err(format!(
                "Ollama returned HTTP {}; check the model and server",
                response.status().as_u16()
            ));
        }
        let mut pending = Vec::new();
        let mut progress = FrameProgress::default();
        loop {
            let chunk = tokio::time::timeout(self.idle_timeout, response.chunk())
                .await
                .map_err(|_| "Ollama stream stalled; partial reply retained")?
                .map_err(|_| "Ollama connection lost; partial reply retained")?;
            let Some(chunk) = chunk else {
                if !pending.is_empty()
                    && self
                        .frame(&pending, session, attempt, sender, &mut progress, trace)
                        .await?
                {
                    return self.complete_recorded(identity, &body, sender, trace).await;
                }
                return Err("Ollama disconnected before completion; partial reply retained".into());
            };
            // Bound incomplete frames even when a server never supplies a newline.
            for byte in chunk {
                if byte == b'\n' {
                    if self
                        .frame(&pending, session, attempt, sender, &mut progress, trace)
                        .await?
                    {
                        return self.complete_recorded(identity, &body, sender, trace).await;
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
