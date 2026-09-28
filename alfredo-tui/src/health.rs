//! Transient model-server health for the header. Polls and preloads bypass
//! inference admission, never block the UI and never become error dialogs.
use crate::provider::Ollama;
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{runtime::Handle, task::JoinHandle};

pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    /// No observation yet, or no monitor.
    Unknown,
    /// Server reachable; the selected model is not resident.
    Up,
    Down {
        since: Instant,
    },
    Loading {
        model: String,
    },
    Ready {
        model: String,
    },
}

impl Health {
    /// Header text; `with_model` includes the model name on wide terminals.
    pub fn label(&self, model: &str, with_model: bool) -> Option<String> {
        let named = |suffix: &str| {
            let mut text = String::from("ollama ✓");
            if with_model {
                text.push(' ');
                text.extend(model.chars().filter(|c| !c.is_control()).take(40));
            }
            if !suffix.is_empty() {
                text.push(' ');
                text.push_str(suffix);
            }
            text
        };
        match self {
            Self::Unknown => None,
            Self::Up => Some(named("")),
            Self::Down { .. } => Some("ollama ✗ retrying".into()),
            Self::Loading { .. } => Some(named("loading")),
            Self::Ready { .. } => Some(named("warm")),
        }
    }
    pub fn healthy(&self) -> bool {
        !matches!(self, Self::Down { .. })
    }
}

#[derive(Default)]
struct State {
    /// None until the first poll completes.
    reachable: Option<Result<(), Instant>>,
    running: Vec<String>,
    preloading: Option<String>,
    preload_error: Option<String>,
    revision: u64,
    stopped: bool,
    fixed: Option<Health>,
}

struct Control {
    handle: Handle,
    provider: Ollama,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    control: Option<Control>,
}

impl Inner {
    fn update(&self, change: impl FnOnce(&mut State)) {
        let mut state = self.state.lock().unwrap();
        change(&mut state);
        state.revision = state.revision.wrapping_add(1);
    }
}

/// Cheap shared reader held by the UI state. The default view is inert.
#[derive(Clone, Default)]
pub struct HealthView {
    inner: Arc<Inner>,
    seen: Cell<u64>,
}

impl HealthView {
    /// A fixed observation with no background work, for rendering.
    pub fn observed(health: Health) -> Self {
        let view = Self::default();
        view.inner.update(|state| {
            state.fixed = Some(health);
            state.stopped = true;
        });
        view
    }

    pub fn state(&self, model: &str) -> Health {
        let state = self.inner.state.lock().unwrap();
        if let Some(fixed) = &state.fixed {
            return fixed.clone();
        }
        match state.reachable {
            None => Health::Unknown,
            Some(Err(since)) => Health::Down { since },
            Some(Ok(())) if state.running.iter().any(|name| name == model) => Health::Ready {
                model: model.into(),
            },
            Some(Ok(())) if state.preloading.as_deref() == Some(model) => Health::Loading {
                model: model.into(),
            },
            Some(Ok(())) => Health::Up,
        }
    }

    pub fn preload_error(&self) -> Option<String> {
        self.inner.state.lock().unwrap().preload_error.clone()
    }

    /// True once the owning monitor is dropped, or when there never was one.
    pub fn stopped(&self) -> bool {
        self.inner.control.is_none() || self.inner.state.lock().unwrap().stopped
    }

    /// True once per observed change; drives redraws.
    pub fn changed(&self) -> bool {
        let revision = self.inner.state.lock().unwrap().revision;
        let changed = revision != self.seen.get();
        self.seen.set(revision);
        changed
    }

    /// Warm `model` in the background. Replaces an earlier preload; failure is
    /// recorded as status only. No-op without a running monitor.
    pub fn preload(&self, model: &str) {
        let Some(control) = &self.inner.control else {
            return;
        };
        let mut tasks = control.tasks.lock().unwrap();
        if self.inner.state.lock().unwrap().stopped {
            return;
        }
        let model = model.to_string();
        self.inner.update(|state| {
            state.preloading = Some(model.clone());
            state.preload_error = None;
        });
        let inner = self.inner.clone();
        let provider = control.provider.clone();
        tasks.retain(|task| !task.is_finished());
        tasks.push(control.handle.spawn(async move {
            let result = provider.preload(&model).await;
            let running = provider.running_models().await.ok();
            inner.update(|state| {
                if state.preloading.as_deref() == Some(model.as_str()) {
                    state.preloading = None;
                }
                match result {
                    Ok(()) => {
                        state.reachable = Some(Ok(()));
                        if let Some(running) = running {
                            state.running = running;
                        } else if !state.running.contains(&model) {
                            state.running.push(model);
                        }
                    }
                    Err(error) => state.preload_error = Some(format!("Preload {model}: {error}")),
                }
            });
        }));
    }
}

/// Owns the poll loop; dropping it stops polling and in-flight preloads.
pub struct Monitor {
    view: HealthView,
}

impl Monitor {
    pub fn start(handle: &Handle, provider: Ollama, interval: Duration) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::default(),
            control: Some(Control {
                handle: handle.clone(),
                provider: provider.clone(),
                tasks: Mutex::default(),
            }),
        });
        let poller = inner.clone();
        let task = handle.spawn(async move {
            loop {
                let observed = provider.running_models().await;
                poller.update(|state| match observed {
                    Ok(running) => {
                        state.reachable = Some(Ok(()));
                        state.running = running;
                    }
                    Err(_) => {
                        if !matches!(state.reachable, Some(Err(_))) {
                            state.reachable = Some(Err(Instant::now()));
                        }
                        state.running.clear();
                    }
                });
                tokio::time::sleep(interval).await;
            }
        });
        if let Some(control) = &inner.control {
            control.tasks.lock().unwrap().push(task);
        }
        Self {
            view: HealthView {
                inner,
                seen: Cell::new(0),
            },
        }
    }

    pub fn view(&self) -> HealthView {
        self.view.clone()
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(control) = &self.view.inner.control {
            // Same lock order as preload(): no task can be added after this.
            let mut tasks = control.tasks.lock().unwrap();
            self.view.inner.update(|state| state.stopped = true);
            for task in tasks.drain(..) {
                task.abort();
            }
        }
    }
}
