//! Discoverable command drafts. Completing a command never executes it.
pub const COMMANDS: &[(&str, &str)] = &[
    (
        "/workspace",
        "switch repository or mission; save current work",
    ),
    (
        "/go",
        "GOAL — autopilot: plan, approve, run, review/repair, integrate",
    ),
    ("/pause", "pause autopilot; running workers finish (F5)"),
    ("/resume", "resume a paused or restored autopilot (F5)"),
    ("/stop", "pause autopilot and cancel running workers"),
    ("/autopilot", "show autopilot status and completion summary"),
    ("/scope", "[JSON] — inspect or draft project understanding"),
    (
        "/scope-confirm",
        "REVISION — confirm the exact reviewed draft",
    ),
    ("/scope-retry", "repeat the exact last scope request"),
    ("/plan", "request — generate a reviewable task draft"),
    ("/plan-revise", "request — refine the current unsaved draft"),
    (
        "/architect-revise",
        "ID — resume pending Architect repair revision",
    ),
    ("/plan-save", "save reviewed draft as proposed tasks"),
    ("/plan-cancel", "stop planning; take no task action"),
    ("/task", "description — propose coding work"),
    ("/tasks", "[query or #ID] — find tasks; empty clears filter"),
    ("/activity", "[query or #ID] — inspect saved task receipts"),
    ("/after", "1,2 description — propose dependent work"),
    ("/permit", "ID JSON — set exact files and check argv"),
    (
        "/assign",
        "ID MODEL — assign installed worker model; clears approval",
    ),
    ("/approve", "[ID] — approve task policy"),
    (
        "/dispatch",
        "on|off — start ready approved work automatically",
    ),
    ("/run", "[ID] — start approved worker"),
    ("/cancel-task", "[ID] — cancel selected task"),
    ("/evidence", "[ID] — inspect verified evidence"),
    ("/recover", "[ID] — recover a stopped run without replay"),
    ("/accept", "[ID] — accept reviewed evidence"),
    (
        "/repair",
        "ID reason — propose a linked repair with inherited policy",
    ),
    (
        "/branch",
        "[ID] — create local review branch for accepted result",
    ),
    (
        "/review",
        "ID JSON — record criterion assessments and review reason",
    ),
    (
        "/resolve-repair",
        "ID — select accepted repair for original dependencies",
    ),
    ("/reject", "[ID] — reject reviewed evidence"),
    ("/refresh", "reload acknowledged task state"),
    ("/retry-task", "repeat exact failed storage request"),
    (
        "/retry-command",
        "SESSION:COMMAND — explicitly retry a saved intent",
    ),
    (
        "/watch",
        "ID|architect — open an agent's transcript (F6, Enter on its row)",
    ),
    (
        "/tell",
        "ID|architect TEXT — instruct an agent: steer, repair, follow up, revise",
    ),
    ("/chat", "return to conversation"),
    ("/models", "list installed models"),
    ("/model", "NAME — select conversation model"),
];

pub const CAPABILITIES: &[(&str, &str)] = &[(
    "@wayfinder",
    "request — discuss project scope; confirmation stays explicit",
)];

/// Only actual native capability routes are admitted; unknown names never go to inference.
pub fn capability_prompt(text: &str) -> Result<Option<&str>, String> {
    let text = text.trim_start();
    if !text.starts_with('@') {
        return Ok(None);
    }
    let (name, prompt) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    if name != "@wayfinder" {
        return Err("Unknown capability; use @wayfinder or F1 for available commands".into());
    }
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Usage: @wayfinder REQUEST".into());
    }
    Ok(Some(prompt))
}

pub struct Choice {
    pub name: String,
    pub description: &'static str,
    /// Help group heading; set only for the F1 catalog.
    pub group: Option<&'static str>,
    draft: String,
}

/// F1 help groups, most common first. Every catalog entry appears exactly once.
pub const HELP_GROUPS: &[(&str, &[&str])] = &[
    (
        "Autopilot",
        &["/go", "/pause", "/resume", "/stop", "/autopilot"],
    ),
    (
        "Tasks",
        &[
            "/tasks",
            "/task",
            "/after",
            "/plan",
            "/plan-revise",
            "/plan-save",
            "/plan-cancel",
            "/approve",
            "/run",
            "/dispatch",
            "/cancel-task",
        ],
    ),
    (
        "Review",
        &[
            "/evidence",
            "/accept",
            "/reject",
            "/repair",
            "/resolve-repair",
            "/review",
            "/branch",
        ],
    ),
    ("Agents", &["/watch", "/tell"]),
    (
        "Chat",
        &[
            "/chat",
            "/model",
            "/models",
            "@wayfinder",
            "/scope",
            "/scope-confirm",
            "/scope-retry",
        ],
    ),
    ("Navigation", &["/workspace", "/activity", "/refresh"]),
    (
        "Advanced",
        &[
            "/permit",
            "/assign",
            "/recover",
            "/architect-revise",
            "/retry-task",
            "/retry-command",
        ],
    ),
];

pub struct Completion {
    pub choices: Vec<Choice>,
    pub selected: usize,
}
impl Completion {
    fn from_choices(choices: Vec<Choice>) -> Option<Self> {
        (!choices.is_empty()).then_some(Self {
            choices,
            selected: 0,
        })
    }
    pub fn open(draft: &str) -> Option<Self> {
        if !(draft.starts_with('/') || draft.starts_with('@'))
            || draft.chars().any(char::is_whitespace)
        {
            return None;
        }
        Self::from_choices(
            (if draft.starts_with('@') {
                CAPABILITIES
            } else {
                COMMANDS
            })
            .iter()
            .filter(|(name, _)| name.starts_with(draft))
            .map(|(name, description)| Choice {
                name: (*name).into(),
                description,
                group: None,
                draft: format!("{name} "),
            })
            .collect(),
        )
    }
    fn model_argument(draft: &str) -> Option<(&str, &str)> {
        let prefix = if let Some(prefix) = draft.strip_prefix("/model ") {
            prefix
        } else {
            let (id, prefix) = draft.strip_prefix("/assign ")?.split_once(' ')?;
            if !id.bytes().all(|c| c.is_ascii_digit()) || id.parse::<u64>().ok()? == 0 {
                return None;
            }
            prefix
        };
        if prefix.chars().any(char::is_whitespace) {
            return None;
        }
        Some((&draft[..draft.len() - prefix.len()], prefix))
    }
    pub fn accepts(draft: &str) -> bool {
        ((draft.starts_with('/') || draft.starts_with('@'))
            && !draft.chars().any(char::is_whitespace))
            || Self::model_argument(draft).is_some()
    }
    pub fn with_models(draft: &str, models: &[String]) -> Option<Self> {
        let Some((head, prefix)) = Self::model_argument(draft) else {
            return Self::open(draft);
        };
        let mut names: Vec<_> = models
            .iter()
            .take(256)
            .filter(|name| {
                !name.is_empty()
                    && name.len() <= 200
                    && !name.chars().any(|c| c.is_control() || c.is_whitespace())
                    && name.starts_with(prefix)
            })
            .collect();
        names.sort();
        names.dedup();
        Self::from_choices(
            names
                .into_iter()
                .map(|name| Choice {
                    name: name.clone(),
                    description: "installed model",
                    group: None,
                    draft: format!("{head}{name}"),
                })
                .collect(),
        )
    }
    /// The grouped F1 catalog: every command and capability, most common first.
    pub fn all() -> Self {
        let mut remaining: Vec<Choice> = Self::open("/")
            .expect("command catalog is nonempty")
            .choices
            .into_iter()
            .chain(
                Self::open("@")
                    .expect("capability catalog is nonempty")
                    .choices,
            )
            .collect();
        let mut choices = Vec::with_capacity(remaining.len());
        for (group, names) in HELP_GROUPS {
            for name in *names {
                if let Some(index) = remaining.iter().position(|choice| choice.name == *name) {
                    let mut choice = remaining.remove(index);
                    choice.group = Some(group);
                    choices.push(choice);
                }
            }
        }
        // Anything not yet grouped still appears, under Advanced.
        choices.extend(remaining.into_iter().map(|mut choice| {
            choice.group = Some("Advanced");
            choice
        }));
        Self {
            choices,
            selected: 0,
        }
    }
    pub fn next(&mut self, forward: bool) {
        self.selected = if forward {
            (self.selected + 1) % self.choices.len()
        } else {
            (self.selected + self.choices.len() - 1) % self.choices.len()
        };
    }
    pub fn draft(&self) -> String {
        self.choices[self.selected].draft.clone()
    }
}
