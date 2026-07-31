use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PROTOTYPE_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Deserialize)]
pub struct LegacyWorkspaceSnapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub workspace_session: LegacyWorkspaceSession,
    pub active_mission: Option<LegacyMissionSummary>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LegacyWorkspaceSession {
    pub workspace_path: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LegacyMissionSummary {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum JourneyPhase {
    SelectionRequired,
    MissionChoiceRequired,
    ReadyForFormation,
    PlanningRequired,
    DispatchReady,
    SessionQueued,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MissionRef {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MissionFormationRoute {
    NoAction,
    AdHocDelegation,
    BoundedDiscovery,
    Wayfinding,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueuedSession {
    pub session_id: String,
    pub mission_id: String,
    pub task: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EffectReceipt {
    pub correlation_id: String,
    pub boundary: String,
    pub revision: u64,
    pub effect_kind: String,
    pub canonical_effect: bool,
    pub replayed: bool,
    pub session_id: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PrototypeState {
    pub schema_version: u32,
    pub revision: u64,
    pub starting_location: String,
    pub coding_workspace: Option<String>,
    pub phase: JourneyPhase,
    pub known_missions: Vec<MissionRef>,
    pub active_mission_id: Option<String>,
    pub formation_route: Option<MissionFormationRoute>,
    pub task: Option<String>,
    pub sessions: Vec<QueuedSession>,
    pub receipts: BTreeMap<String, EffectReceipt>,
}

impl PrototypeState {
    pub fn selection_required(starting_location: impl Into<String>) -> Self {
        Self {
            schema_version: PROTOTYPE_SCHEMA_VERSION,
            revision: 0,
            starting_location: starting_location.into(),
            coding_workspace: None,
            phase: JourneyPhase::SelectionRequired,
            known_missions: Vec::new(),
            active_mission_id: None,
            formation_route: None,
            task: None,
            sessions: Vec::new(),
            receipts: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    SelectWorkspace {
        correlation_id: String,
        expected_revision: u64,
        path: String,
        repository_valid: bool,
        known_missions: Vec<MissionRef>,
    },
    ChooseMission {
        correlation_id: String,
        expected_revision: u64,
        choice: MissionChoice,
    },
    FormMission {
        correlation_id: String,
        expected_revision: u64,
        route: MissionFormationRoute,
        task: String,
        effectful_request: bool,
        controller_claims_effect: bool,
    },
    Dispatch {
        correlation_id: String,
        expected_revision: u64,
        proposal_exact: bool,
        worker_eligible: bool,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "choice", rename_all = "kebab-case")]
pub enum MissionChoice {
    Resume { mission_id: String },
    StartNew { mission_id: String, title: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Transition {
    pub state: PrototypeState,
    pub receipt: EffectReceipt,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DecisionFailure {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
}

impl DecisionFailure {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            recoverable: true,
        }
    }
}

pub fn import_legacy_v1(
    source: &str,
    starting_location: impl Into<String>,
) -> Result<PrototypeState, DecisionFailure> {
    let legacy: LegacyWorkspaceSnapshot = serde_json::from_str(source).map_err(|error| {
        DecisionFailure::new(
            "contract-failure",
            format!("schema-v1 Workspace Snapshot could not be decoded: {error}"),
        )
    })?;
    if legacy.schema_version != 1 {
        return Err(DecisionFailure::new(
            "contract-failure",
            format!(
                "expected the current Workspace Snapshot schema 1, received {}",
                legacy.schema_version
            ),
        ));
    }
    if legacy.workspace_session.workspace_path.trim().is_empty() {
        return Err(DecisionFailure::new(
            "workspace-invalid",
            "schema-v1 state did not contain a selected Coding Workspace",
        ));
    }
    let active_mission_id = legacy
        .active_mission
        .as_ref()
        .map(|mission| mission.id.clone());
    let known_missions = legacy
        .active_mission
        .map(|mission| {
            vec![MissionRef {
                id: mission.id,
                title: mission.title,
            }]
        })
        .unwrap_or_default();
    Ok(PrototypeState {
        schema_version: PROTOTYPE_SCHEMA_VERSION,
        revision: legacy.revision,
        starting_location: starting_location.into(),
        coding_workspace: Some(legacy.workspace_session.workspace_path),
        phase: if active_mission_id.is_some() {
            JourneyPhase::ReadyForFormation
        } else {
            JourneyPhase::MissionChoiceRequired
        },
        known_missions,
        active_mission_id,
        formation_route: None,
        task: None,
        sessions: Vec::new(),
        receipts: BTreeMap::new(),
    })
}

impl Action {
    fn correlation_id(&self) -> &str {
        match self {
            Self::SelectWorkspace { correlation_id, .. }
            | Self::ChooseMission { correlation_id, .. }
            | Self::FormMission { correlation_id, .. }
            | Self::Dispatch { correlation_id, .. } => correlation_id,
        }
    }

    fn expected_revision(&self) -> u64 {
        match self {
            Self::SelectWorkspace {
                expected_revision, ..
            }
            | Self::ChooseMission {
                expected_revision, ..
            }
            | Self::FormMission {
                expected_revision, ..
            }
            | Self::Dispatch {
                expected_revision, ..
            } => *expected_revision,
        }
    }

    fn boundary(&self) -> Result<String, DecisionFailure> {
        serde_json::to_string(self).map_err(|error| {
            DecisionFailure::new(
                "contract-failure",
                format!("action boundary could not be serialized: {error}"),
            )
        })
    }
}

pub fn apply(state: &PrototypeState, action: Action) -> Result<Transition, DecisionFailure> {
    if state.schema_version != PROTOTYPE_SCHEMA_VERSION {
        return Err(DecisionFailure::new(
            "contract-failure",
            "unsupported prototype state schema",
        ));
    }
    let boundary = action.boundary()?;
    let correlation_id = action.correlation_id().to_owned();
    if let Some(existing) = state.receipts.get(&correlation_id) {
        if existing.boundary != boundary {
            return Err(DecisionFailure::new(
                "correlation-conflict",
                "the correlation id was already used for a different boundary",
            ));
        }
        let mut replay = existing.clone();
        replay.replayed = true;
        return Ok(Transition {
            state: state.clone(),
            receipt: replay,
        });
    }
    if action.expected_revision() != state.revision {
        return Err(DecisionFailure::new(
            "stale-action",
            format!(
                "expected revision {}, current revision is {}",
                action.expected_revision(),
                state.revision
            ),
        ));
    }

    let mut next = state.clone();
    let (effect_kind, canonical_effect, session_id, message) = match action {
        Action::SelectWorkspace {
            path,
            repository_valid,
            known_missions,
            ..
        } => {
            if next.phase != JourneyPhase::SelectionRequired || next.coding_workspace.is_some() {
                return Err(DecisionFailure::new(
                    "workspace-already-selected",
                    "a Coding Workspace is already bound",
                ));
            }
            if !repository_valid || path.trim().is_empty() {
                return Err(DecisionFailure::new(
                    "workspace-invalid",
                    "the selected path was not acknowledged as a repository",
                ));
            }
            next.coding_workspace = Some(path);
            next.known_missions = known_missions;
            next.phase = JourneyPhase::MissionChoiceRequired;
            (
                "workspace-selected".to_owned(),
                true,
                None,
                "Coding Workspace selected; Mission choice is still required.".to_owned(),
            )
        }
        Action::ChooseMission { choice, .. } => {
            if next.phase != JourneyPhase::MissionChoiceRequired || next.coding_workspace.is_none()
            {
                return Err(DecisionFailure::new(
                    "mission-choice-ineligible",
                    "select a Coding Workspace before choosing a Mission",
                ));
            }
            let mission_id = match choice {
                MissionChoice::Resume { mission_id } => {
                    if !next
                        .known_missions
                        .iter()
                        .any(|mission| mission.id == mission_id)
                    {
                        return Err(DecisionFailure::new(
                            "mission-not-found",
                            "the requested Mission is not known for this Coding Workspace",
                        ));
                    }
                    mission_id
                }
                MissionChoice::StartNew { mission_id, title } => {
                    if next
                        .known_missions
                        .iter()
                        .any(|mission| mission.id == mission_id)
                    {
                        return Err(DecisionFailure::new(
                            "mission-duplicate",
                            "Start New Mission cannot silently reuse a known Mission id",
                        ));
                    }
                    next.known_missions.push(MissionRef {
                        id: mission_id.clone(),
                        title,
                    });
                    mission_id
                }
            };
            next.active_mission_id = Some(mission_id);
            next.phase = JourneyPhase::ReadyForFormation;
            (
                "mission-selected".to_owned(),
                true,
                None,
                "Mission choice acknowledged; no work has been dispatched.".to_owned(),
            )
        }
        Action::FormMission {
            route,
            task,
            effectful_request,
            controller_claims_effect,
            ..
        } => {
            if next.phase != JourneyPhase::ReadyForFormation || next.active_mission_id.is_none() {
                return Err(DecisionFailure::new(
                    "formation-ineligible",
                    "an acknowledged Coding Workspace and Mission are required",
                ));
            }
            if task.trim().is_empty() {
                return Err(DecisionFailure::new(
                    "contract-failure",
                    "Mission Formation requires a non-empty goal",
                ));
            }
            next.task = Some(task);
            next.formation_route = Some(route.clone());
            match route {
                MissionFormationRoute::NoAction => {
                    next.phase = JourneyPhase::ReadyForFormation;
                    let message = if effectful_request || controller_claims_effect {
                        "No action taken. Controller prose is commentary and no Orchestrator effect receipt exists."
                    } else {
                        "Discussion recorded; no action was requested or taken."
                    };
                    ("no-action".to_owned(), false, None, message.to_owned())
                }
                MissionFormationRoute::AdHocDelegation => {
                    next.phase = JourneyPhase::DispatchReady;
                    (
                        "formation-route-selected".to_owned(),
                        true,
                        None,
                        "Ad Hoc Delegation selected; exact approval and worker eligibility are still required."
                            .to_owned(),
                    )
                }
                MissionFormationRoute::BoundedDiscovery => {
                    next.phase = JourneyPhase::PlanningRequired;
                    (
                        "formation-route-selected".to_owned(),
                        true,
                        None,
                        "Bounded discovery selected; continue with live grilling before publication."
                            .to_owned(),
                    )
                }
                MissionFormationRoute::Wayfinding => {
                    next.phase = JourneyPhase::PlanningRequired;
                    (
                        "formation-route-selected".to_owned(),
                        true,
                        None,
                        "Wayfinding selected; no Local Agent session was dispatched.".to_owned(),
                    )
                }
            }
        }
        Action::Dispatch {
            proposal_exact,
            worker_eligible,
            ..
        } => {
            if next.phase != JourneyPhase::DispatchReady
                || next.formation_route != Some(MissionFormationRoute::AdHocDelegation)
            {
                return Err(DecisionFailure::new(
                    "dispatch-ineligible",
                    "only an acknowledged Ad Hoc Delegation route may dispatch",
                ));
            }
            if !proposal_exact {
                return Err(DecisionFailure::new(
                    "proposal-mismatch",
                    "the canonical proposal no longer matches the approved boundary",
                ));
            }
            if !worker_eligible {
                return Err(DecisionFailure::new(
                    "worker-ineligible",
                    "the selected worker is not currently eligible",
                ));
            }
            let session_id = format!("rust-prototype-session-{:04}", next.sessions.len() + 1);
            next.sessions.push(QueuedSession {
                session_id: session_id.clone(),
                mission_id: next.active_mission_id.clone().unwrap_or_default(),
                task: next.task.clone().unwrap_or_default(),
                status: "queued".to_owned(),
            });
            next.phase = JourneyPhase::SessionQueued;
            (
                "session-queued".to_owned(),
                true,
                Some(session_id),
                "One Local Agent session is durably eligible to queue; execution remains deferred."
                    .to_owned(),
            )
        }
    };

    next.revision += 1;
    let receipt = EffectReceipt {
        correlation_id: correlation_id.clone(),
        boundary,
        revision: next.revision,
        effect_kind,
        canonical_effect,
        replayed: false,
        session_id,
        message,
    };
    next.receipts.insert(correlation_id, receipt.clone());
    Ok(Transition {
        state: next,
        receipt,
    })
}
