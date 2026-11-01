use super::{Action, NodeId, Projection, Refusal, Request, Runtime, TaskControl};

impl TaskControl {
    /// Capture targets and revisions now. This method never starts work or writes state.
    pub fn prepare_command(
        &mut self,
        text: &str,
        model: &str,
    ) -> Result<Option<crate::command_intent::Intent>, String> {
        use crate::command_intent::Intent;
        let mut text = text.trim().to_string();
        if matches!(
            text.as_str(),
            "/approve"
                | "/branch"
                | "/run"
                | "/cancel-task"
                | "/evidence"
                | "/accept"
                | "/reject"
                | "/recover"
        ) {
            text = format!(
                "{} {}",
                text,
                self.selected_task().ok_or("Select a task first")?.id
            );
        }
        let verb = text.split_whitespace().next().unwrap_or("");
        if matches!(
            verb,
            "/tasks" | "/activity" | "/chat" | "/refresh" | "/evidence"
        ) || text == "/scope"
            || text == "/plan"
        {
            return Ok(None);
        }
        if verb == "/dispatch" {
            let enabled = match text.as_str() {
                "/dispatch on" => true,
                "/dispatch off" => false,
                _ => return Err("Usage: /dispatch on|off".into()),
            };
            let scope_revision_for_on = if enabled {
                if self.scope_status.blocked {
                    return Err(self.scope_status.label.clone());
                }
                Some(
                    self.scope_status
                        .revision
                        .ok_or("Refresh scope before enabling dispatch")?,
                )
            } else {
                None
            };
            return self
                .prepare_control(crate::control_command::Operation::Dispatch {
                    enabled,
                    expected_epoch: self.controller_epoch,
                    scope_revision_for_on,
                })
                .map(Some);
        }
        if let Some(id) = text
            .strip_prefix("/cancel-task ")
            .and_then(|id| id.parse::<u64>().ok())
        {
            if let Some(identity) = self.worker_identities.get(&id) {
                return self
                    .prepare_control(crate::control_command::Operation::CancelWorker {
                        task: id,
                        start_correlation: identity.correlation.clone(),
                        expected_start_revision: identity.start_revision,
                    })
                    .map(Some);
            }
        }
        if self.pending {
            return Err("Wait for the pending task acknowledgment".into());
        }
        if text == "/retry-task" {
            return self
                .prepared_retry
                .clone()
                .or_else(|| self.retry.clone().map(|request| Intent::Task { request }))
                .map(Some)
                .ok_or("No saved task intent to retry".into());
        }
        if text == "/scope-retry" {
            return self
                .scope_retry
                .clone()
                .map(|request| Some(Intent::Scope { request }))
                .ok_or("No scope request to retry".into());
        }
        self.sequence += 1;
        let correlation = format!("{}-{}", self.incarnation, self.sequence);
        if matches!(
            verb,
            "/plan" | "/plan-revise" | "/architect-revise" | "/plan-cancel"
        ) {
            let snapshot = self.snapshot.as_ref().ok_or("Refresh task state first")?;
            return self
                .planner
                .prepare_command(&correlation, &text, model, snapshot)
                .map(|request| request.map(|request| Intent::Planner { request }));
        }
        let intent = if text.starts_with("/scope ") || text.starts_with("/scope-confirm ") {
            let revision = self
                .scope_view
                .as_ref()
                .ok_or("Open /scope and review the current state first")?
                .revision;
            let action = if let Some(json) = text.strip_prefix("/scope ") {
                crate::understanding::Action::Draft {
                    brief: serde_json::from_str(json).map_err(|_| "Invalid scope draft JSON")?,
                }
            } else {
                crate::understanding::Action::Confirm {
                    draft_revision: text
                        .strip_prefix("/scope-confirm ")
                        .unwrap()
                        .parse()
                        .map_err(|_| USAGE_SCOPE_CONFIRM)?,
                }
            };
            Intent::Scope {
                request: crate::understanding::Request {
                    correlation,
                    expected_revision: revision,
                    action,
                },
            }
        } else {
            let snapshot = self
                .snapshot
                .as_ref()
                .ok_or("Task queue unavailable; /refresh first")?;
            if matches!(verb, "/run" | "/branch" | "/recover") {
                let argument = text.split_once(' ').ok_or("Command needs a task ID")?.1;
                let (argument, requeue) = match argument.strip_suffix(" requeue") {
                    Some(id) if verb == "/recover" => (id, true),
                    _ => (argument, false),
                };
                let task: u64 = argument.parse().map_err(|_| "Invalid task ID")?;
                let item = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == task)
                    .ok_or_else(|| crate::tasks::unknown_task(task))?;
                match verb {
                    "/run" => Intent::Run {
                        correlation,
                        expected_revision: snapshot.revision,
                        task,
                    },
                    "/branch" => {
                        let existing = snapshot.receipts.iter().find(|receipt| matches!(receipt.request.action, Action::Branch { task: id, .. } if id == task));
                        Intent::Branch {
                            correlation: existing
                                .map_or(correlation, |receipt| receipt.request.correlation.clone()),
                            expected_revision: existing.map_or(snapshot.revision, |receipt| {
                                receipt.request.expected_revision
                            }),
                            task,
                            run: item
                                .run
                                .as_ref()
                                .ok_or_else(|| crate::tasks::not_run(item))?
                                .id
                                .clone(),
                        }
                    }
                    _ => Intent::Recover {
                        correlation,
                        task,
                        run: item
                            .run
                            .as_ref()
                            .ok_or_else(|| crate::tasks::not_run(item))?
                            .id
                            .clone(),
                        requeue,
                    },
                }
            } else if text == "/plan-save" {
                if self.planner.active() {
                    return Err("Wait for the complete plan before saving".into());
                }
                Intent::Task {
                    request: Request {
                        correlation,
                        expected_revision: self.planner.revision,
                        action: Action::Plan {
                            plan: self
                                .planner
                                .draft_for_save()
                                .ok_or("No complete validated plan to save")?,
                        },
                    },
                }
            } else {
                Intent::Task {
                    request: Request {
                        correlation,
                        expected_revision: snapshot.revision,
                        action: parse(&text, model).map_err(|error| {
                            // Only rewrite into a template when the user named a task;
                            // bare `/review` keeps its plain usage line.
                            let task = error
                                .strip_prefix("Usage: /review ")
                                .and_then(|_| text.trim().strip_prefix("/review "))
                                .and_then(|rest| rest.split(' ').next())
                                .and_then(|id| id.parse().ok());
                            match task {
                                Some(task) => {
                                    review_usage(task, snapshot.acceptance_for_task(task).len())
                                }
                                None => error,
                            }
                        })?,
                    },
                }
            }
        };
        intent.validate()?;
        Ok(Some(intent))
    }
    /// Only the caller's durable-save acknowledgment may call this for a new intent.
    pub fn dispatch_prepared(
        &mut self,
        runtime: &Runtime,
        intent: &crate::command_intent::Intent,
    ) -> Result<(), String> {
        use crate::command_intent::Intent;
        intent.validate()?;
        if intent.selection_request().is_some() {
            return Err("Selection must pass the workspace journal and handoff boundary".into());
        }
        if matches!(intent, Intent::Wayfinder { .. }) {
            return Err("Wayfinder turns require the saved Router dispatch boundary".into());
        }
        if let Intent::Control { request } = intent {
            self.prepared_retry = Some(intent.clone());
            return self.dispatch_control(runtime, request);
        }
        if let Intent::DispatchRun { request } = intent {
            return self.dispatch_automatic(runtime, request);
        }
        if let Intent::ArchitectDraft { request } = intent {
            return self.dispatch_architect(runtime, request);
        }
        if self.pending || self.intent_pending(intent) {
            return Err("Command acknowledgment is already pending".into());
        }
        self.prepared_errors.remove(intent.correlation());
        self.prepared_transient.remove(intent.correlation());
        self.prepared_refused.remove(intent.correlation());
        self.prepared_retry = Some(intent.clone());
        self.visible = true;
        let target = match intent {
            Intent::Task {
                request:
                    Request {
                        action:
                            Action::Review { task, .. }
                            | Action::Assess { task, .. }
                            | Action::Decide { task, .. }
                            | Action::ReviewAndRepair { task, .. }
                            | Action::ReviewArchitecture { task, .. },
                        ..
                    },
            } => Some(*task),
            _ => None,
        };
        if self
            .evidence
            .as_ref()
            .is_none_or(|view| Some(view.task) != target)
        {
            self.evidence = None;
        }
        self.activity = None;
        self.planner.visible = false;
        match intent {
            Intent::Selection { .. }
            | Intent::SelectionArrival { .. }
            | Intent::Control { .. }
            | Intent::DispatchRun { .. }
            | Intent::ArchitectDraft { .. }
            | Intent::Wayfinder { .. } => {
                unreachable!("controller commands dispatch before task admission")
            }
            Intent::Planner { request } => {
                let provider = self
                    .provider
                    .clone()
                    .ok_or("Planner provider unavailable")?;
                self.planner
                    .dispatch_command(runtime, provider, request, self.store.clone())?;
                self.scroll = 0;
            }
            Intent::Task { request } => {
                self.retry = Some(request.clone());
                self.architecture_launch =
                    matches!(request.action, Action::ReviewArchitecture { .. })
                        .then(|| request.clone());
                self.prepared_active = Some(intent.clone());
                self.launch(runtime, Some(request.clone()));
            }
            Intent::Run {
                correlation,
                expected_revision,
                task,
            } => {
                self.start_worker_prepared(
                    runtime,
                    *task,
                    *expected_revision,
                    correlation.clone(),
                )?;
                self.prepared_workers.insert(*task, intent.clone());
            }
            Intent::Scope { request } => {
                self.scope_retry = Some(request.clone());
                let request = request.clone();
                let store = self.store.clone();
                let sender = self.sender.clone();
                self.pending = true;
                self.writing = true;
                self.prepared_active = Some(intent.clone());
                runtime.spawn(async move {
                    let result = tokio::task::spawn_blocking(move || {
                        // The scope store mixes validation and write failures, so a
                        // failure here is reported as uncertain rather than as refused.
                        let state = store
                            .understanding()
                            .transact(request)
                            .map_err(Refusal::Uncertain)?;
                        Ok(Projection {
                            snapshot: store.snapshot().map_err(Refusal::Uncertain)?,
                            notice: format!("Scope receipt saved · revision {}", state.revision),
                            evidence: None,
                            run_observations: None,
                            scope: Some(state.clone()),
                            canonical_scope: Some(state.clone()),
                            gate: Some(crate::task_view::ScopeStatus::observe(Ok(state))),
                        })
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(Refusal::Uncertain(
                            "Scope worker stopped; outcome unknown".into(),
                        ))
                    });
                    let _ = sender.send(result).await;
                });
            }
            Intent::Branch {
                correlation,
                expected_revision,
                task,
                run,
            } => {
                let (task, expected_revision, correlation, run) =
                    (*task, *expected_revision, correlation.clone(), run.clone());
                let store = self.store.clone();
                let sender = self.sender.clone();
                self.pending = true;
                self.writing = true;
                self.prepared_active = Some(intent.clone());
                runtime.spawn(async move {
                    let check_store = store.clone();
                    let checked =
                        tokio::task::spawn_blocking(move || {
                            let snapshot = check_store.snapshot()?;
                            if !snapshot.tasks.iter().any(|t| {
                                t.id == task && t.run.as_ref().is_some_and(|r| r.id == run)
                            }) {
                                return Err("Branch source run changed".to_string());
                            }
                            Ok(())
                        })
                        .await;
                    let result = match checked {
                        Ok(Ok(())) => crate::branch::publish_checked(
                            store,
                            task,
                            expected_revision,
                            correlation,
                        )
                        .await
                        .map(Projection::from),
                        Ok(Err(error)) => Err(Refusal::from(error)),
                        Err(_) => Err(Refusal::Uncertain(
                            "Branch verifier stopped; outcome unknown".into(),
                        )),
                    };
                    let _ = sender.send(result).await;
                });
            }
            Intent::Recover {
                task, run, requeue, ..
            } => {
                let (task, run, requeue) = (*task, run.clone(), *requeue);
                let store = self.store.clone();
                let sender = self.sender.clone();
                self.pending = true;
                self.writing = true;
                self.prepared_active = Some(intent.clone());
                runtime.spawn(async move {
                    let result =
                        match tokio::task::spawn_blocking(move || {
                            let snapshot = store.snapshot()?;
                            if !snapshot.tasks.iter().any(|t| {
                                t.id == task && t.run.as_ref().is_some_and(|r| r.id == run)
                            }) {
                                return Err("Recovery source run changed".into());
                            }
                            if requeue {
                                store.recover_requeue_checked(task)
                            } else {
                                store.recover_checked(task)
                            }
                            .map(Projection::from)
                        })
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => Err(Refusal::Uncertain(
                                "Recovery worker stopped; outcome unknown".into(),
                            )),
                        };
                    let _ = sender.send(result).await;
                });
            }
        }
        Ok(())
    }

    pub fn command(&mut self, runtime: &Runtime, text: &str, model: &str) -> Result<(), String> {
        let selected_command;
        let text = if matches!(
            text.trim(),
            "/approve"
                | "/branch"
                | "/run"
                | "/cancel-task"
                | "/evidence"
                | "/accept"
                | "/reject"
                | "/recover"
        ) {
            let id = self.selected_task().ok_or("Select a task first")?.id;
            selected_command = format!("{} {id}", text.trim());
            &selected_command
        } else {
            text
        };
        self.visible = true;
        self.autopilot_report = None;
        let mut words = text.split_whitespace();
        let review_target = match words.next() {
            Some("/review" | "/accept" | "/reject") => {
                words.next().and_then(|id| id.parse::<u64>().ok())
            }
            Some("/retry-task") => self
                .retry
                .as_ref()
                .and_then(|request| match request.action {
                    Action::Assess { task, .. }
                    | Action::Decide { task, .. }
                    | Action::ReviewAndRepair { task, .. }
                    | Action::ReviewArchitecture { task, .. }
                    | Action::Review { task, .. } => Some(task),
                    _ => None,
                }),
            _ => None,
        };
        if self
            .evidence
            .as_ref()
            .is_none_or(|view| Some(view.task) != review_target)
        {
            self.evidence = None;
            self.pending_evidence = None;
        }
        if text.trim() == "/scope"
            || text.trim().starts_with("/scope ")
            || text.trim().starts_with("/scope-confirm ")
            || text.trim() == "/scope-retry"
        {
            if self.pending {
                return Err("Wait for the pending acknowledgment".into());
            }
            let text = text.trim();
            let request = if text == "/scope" {
                None
            } else if text == "/scope-retry" {
                Some(
                    self.scope_retry
                        .clone()
                        .ok_or("No scope request to retry")?,
                )
            } else {
                let revision = self
                    .scope_view
                    .as_ref()
                    .ok_or("Open /scope and review the current state first")?
                    .revision;
                let action = if let Some(json) = text.strip_prefix("/scope ") {
                    crate::understanding::Action::Draft {
                        brief: serde_json::from_str(json).map_err(|_| {
                            "Usage: /scope JSON with destination, scope, constraints, uncertainty"
                        })?,
                    }
                } else {
                    crate::understanding::Action::Confirm {
                        draft_revision: text
                            .strip_prefix("/scope-confirm ")
                            .unwrap()
                            .parse()
                            .map_err(|_| USAGE_SCOPE_CONFIRM)?,
                    }
                };
                self.sequence += 1;
                Some(crate::understanding::Request {
                    correlation: format!("{}-scope-{}", self.incarnation, self.sequence),
                    expected_revision: revision,
                    action,
                })
            };
            if request.is_some() {
                self.scope_retry = request.clone();
            }
            self.pending = true;
            self.writing = request.is_some();
            self.activity = None;
            self.planner.visible = false;
            self.scroll = 0;
            self.notice = "Shared Understanding · awaiting saved state".into();
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let scope = store.understanding();
                    let state = match request {
                        Some(request) => scope.transact(request)?,
                        None => scope.snapshot()?,
                    };
                    Ok(Projection {
                        snapshot: store.snapshot()?,
                        notice: format!(
                            "Shared Understanding observed · revision {} · no task action taken",
                            state.revision
                        ),
                        scope: Some(state.clone()),
                        canonical_scope: Some(state.clone()),
                        evidence: None,
                        gate: Some(crate::task_view::ScopeStatus::observe(Ok(state))),
                        run_observations: None,
                    })
                })
                .await
                .unwrap_or_else(|_| {
                    Err("Understanding worker stopped; retry the exact request".into())
                });
                let _ = sender.send(result).await;
            });
            return Ok(());
        }
        self.scope_view = None;
        if text.trim() == "/activity" || text.trim().starts_with("/activity ") {
            let query = text.trim().strip_prefix("/activity").unwrap().trim();
            if query.len() > 200 || query.chars().any(char::is_control) {
                return Err("Activity query must be at most 200 bytes without controls".into());
            }
            self.planner.visible = false;
            self.activity = Some(query.into());
            self.scroll = 0;
            self.refresh(runtime);
            return Ok(());
        }
        if text.trim() != "/refresh" {
            self.activity = None;
        }
        if let Some(id) = text
            .trim()
            .strip_prefix("/cancel-task ")
            .and_then(|id| id.parse::<u64>().ok())
        {
            if self.worker_identities.contains_key(&id) {
                let intent = self
                    .prepare_command(text, model)?
                    .ok_or("Worker control unavailable")?;
                return self.dispatch_prepared(runtime, &intent);
            }
        }
        if let Some(query) = text.trim().strip_prefix("/tasks ") {
            if self.pending {
                return Err("Wait for the pending task receipt before changing the filter".into());
            }
            let query = query.trim();
            if query.len() > 200 || query.chars().any(char::is_control) {
                return Err("Task query must be at most 200 bytes without controls".into());
            }
            self.remember_initial_work_focus();
            self.task_query = query.into();
            if let Some(task) = query
                .strip_prefix('#')
                .and_then(|id| id.parse::<u64>().ok())
                .filter(|id| *id > 0)
            {
                self.reveal_work_task(task);
                self.focus_work_node(NodeId::Task(task));
            }
            self.planner.visible = false;
            self.scroll = 0;
            self.notice = "Task filter applied · /tasks clears it · no task action taken".into();
            return Ok(());
        }
        match text.trim() {
            "/tasks" => {
                self.remember_initial_work_focus();
                self.task_query.clear();
                self.planner.visible = false;
                self.refresh(runtime);
                return Ok(());
            }
            "/chat" => {
                self.set_visible(false);
                return Ok(());
            }
            "/refresh" => {
                self.refresh(runtime);
                return Ok(());
            }
            _ => {}
        }
        if text.trim().starts_with("/dispatch") {
            let intent = self
                .prepare_command(text, model)?
                .ok_or("Dispatch control unavailable")?;
            return self.dispatch_prepared(runtime, &intent);
        }
        if self.pending {
            return Err("Wait for the pending task receipt".into());
        }
        if let Some(id) = text.trim().strip_prefix("/architect-revise ") {
            let task = id.parse().map_err(|_| USAGE_ARCHITECT_REVISE)?;
            let provider = self
                .provider
                .clone()
                .ok_or("Planner provider unavailable")?;
            self.planner
                .revise_architecture(runtime, provider, task, self.store.clone())?;
            self.scroll = 0;
            return Ok(());
        }
        if text.trim() == "/plan-cancel" {
            let nothing =
                !self.planner.active() && self.planner.draft.is_none() && !self.planner.visible;
            if nothing {
                return Err("No plan draft to cancel".into());
            }
            let discarded_only = !self.planner.active() && self.planner.draft.is_none();
            self.planner.cancel();
            self.planner.draft = None;
            self.planner.visible = false;
            if discarded_only {
                // An invalid or failed draft: close its pane and confirm.
                self.planner.partial.clear();
                self.planner.notice = "Plan draft discarded".into();
            } else {
                self.planner.notice = "Plan cancelled · no action taken".into();
            }
            return Ok(());
        }
        if text.trim() == "/plan" {
            if self.planner.notice.is_empty() {
                return Err("Usage: /plan REQUEST".into());
            }
            self.planner.visible = true;
            self.scroll = 0;
            return Ok(());
        }
        if let Some(prompt) = text.trim().strip_prefix("/plan ") {
            let revision = self
                .snapshot
                .as_ref()
                .ok_or("Refresh task state first")?
                .revision;
            let provider = self
                .provider
                .clone()
                .ok_or("Planner provider unavailable")?;
            self.evidence = None;
            self.scroll = 0;
            self.planner.start(
                runtime,
                provider,
                prompt,
                model,
                revision,
                self.store.clone(),
            )?;
            return Ok(());
        }
        if text.trim() == "/plan-revise" {
            return Err("Usage: /plan-revise REQUEST".into());
        }
        if let Some(request) = text.trim().strip_prefix("/plan-revise ") {
            let revision = self
                .snapshot
                .as_ref()
                .ok_or("Refresh task state first")?
                .revision;
            let provider = self
                .provider
                .clone()
                .ok_or("Planner provider unavailable")?;
            self.planner
                .revise(runtime, provider, request, revision, self.store.clone())?;
            self.evidence = None;
            self.scroll = 0;
            return Ok(());
        }
        if text.trim() == "/plan-save" {
            if self.planner.active() {
                return Err("Wait for the complete plan before saving".into());
            }
            let plan = self
                .planner
                .draft_for_save()
                .ok_or("No complete validated plan to save")?;
            self.sequence += 1;
            let request = Request {
                correlation: format!("{}-{}", self.incarnation, self.sequence),
                expected_revision: self.planner.revision,
                action: Action::Plan { plan },
            };
            self.retry = Some(request.clone());
            self.launch(runtime, Some(request));
            return Ok(());
        }

        if let Some(id) = text.trim().strip_prefix("/branch ") {
            let task = id.parse().map_err(|_| "Usage: /branch ID")?;
            let revision = self
                .snapshot
                .as_ref()
                .ok_or("Refresh task state first")?
                .revision;
            self.sequence += 1;
            let correlation = format!("{}-{}", self.incarnation, self.sequence);
            self.pending = true;
            self.writing = true;
            self.retry = None;
            self.notice = format!("Verifying review branch for task #{task}");
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = crate::branch::publish_checked(store, task, revision, correlation)
                    .await
                    .map(Projection::from);
                let _ = sender.send(result).await;
            });
            return Ok(());
        }
        if let Some(id) = text.trim().strip_prefix("/recover ") {
            let task: u64 = id.parse().map_err(|_| "Usage: /recover ID")?;
            self.pending = true;
            self.writing = true;
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = match tokio::task::spawn_blocking(move || {
                    store.recover_checked(task).map(Projection::from)
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(Refusal::Uncertain(
                        "Recovery stopped; refresh before retrying /recover".into(),
                    )),
                };
                let _ = sender.send(result).await;
            });
            return Ok(());
        }
        if let Some(id) = text.trim().strip_prefix("/evidence ") {
            let task: u64 = id.parse().map_err(|_| "Usage: /evidence ID")?;
            if self.pending {
                return Err("Wait for the pending acknowledgment before opening evidence".into());
            }
            self.pending = true;
            self.writing = false;
            self.sequence += 1;
            let evidence_request = self.sequence;
            self.pending_evidence = Some(evidence_request);
            self.scroll = 0;
            let store = self.store.clone();
            let sender = self.sender.clone();
            runtime.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    let snapshot = store.snapshot()?;
                    let raw = store.evidence(task)?;
                    let current = snapshot
                        .tasks
                        .iter()
                        .find(|item| item.id == task)
                        .ok_or_else(|| crate::tasks::unknown_task(task))?;
                    let evidence = crate::review::View::from_verified_for(current, &raw)?
                        .with_acceptance(snapshot.acceptance_for_task(task))
                        .with_review(snapshot.review_summary_for_task(task).as_deref())
                        .with_inputs(
                            snapshot
                                .tasks
                                .iter()
                                .find(|item| item.id == task)
                                .and_then(|item| item.run.as_ref())
                                .map(|run| run.inputs.as_slice())
                                .unwrap_or_default(),
                        );
                    Ok(Projection {
                        snapshot,
                        notice: format!(
                            "Verified evidence for task #{task}; /tasks returns to queue"
                        ),
                        evidence: Some((evidence_request, evidence)),
                        scope: None,
                        canonical_scope: None,
                        gate: None,
                        run_observations: None,
                    })
                })
                .await
                .unwrap_or_else(|_| Err("Evidence reader stopped".into()));
                let _ = sender.send(result).await;
            });
            return Ok(());
        }
        if let Some(id) = text.trim().strip_prefix("/run ") {
            return self.start_worker(runtime, id.parse().map_err(|_| "Usage: /run ID")?);
        }

        if text.trim() == "/retry-task" {
            let request = self
                .retry
                .clone()
                .ok_or("No failed task request to retry")?;
            self.launch(runtime, Some(request));
            return Ok(());
        }
        let action = parse(text, model)?;
        self.planner.visible = false;
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or("Task queue unavailable; /refresh first")?;
        self.sequence += 1;
        let request = Request {
            correlation: format!("{}-{}", self.incarnation, self.sequence),
            expected_revision: snapshot.revision,
            action,
        };
        self.architecture_launch =
            matches!(request.action, Action::ReviewArchitecture { .. }).then(|| request.clone());
        self.retry = Some(request.clone());
        self.launch(runtime, Some(request));
        Ok(())
    }
}

/// `/review` usage with a copy-pasteable template for the task's criteria.
fn review_usage(task: u64, criteria: usize) -> String {
    format!("Usage: {}", crate::tasks::review_template(task, criteria))
}

pub fn parse(text: &str, model: &str) -> Result<Action, String> {
    if let Some(arguments) = text.trim().strip_prefix("/assign ") {
        let (id, model) = arguments.split_once(' ').ok_or(USAGE_ASSIGN)?;
        let task = id.parse().map_err(|_| "Use a numeric task id")?;
        let model = model.trim();
        if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
            return Err("Invalid worker model identity".into());
        }
        return Ok(Action::Assign {
            task,
            model: model.into(),
        });
    }

    let text = text.trim();
    if let Some(id) = text.strip_prefix("/resolve-repair ") {
        return Ok(Action::ResolveRepair {
            task: id.trim().parse().map_err(|_| USAGE_RESOLVE_REPAIR)?,
        });
    }
    if let Some(id) = text.strip_prefix("/requeue ") {
        return Ok(Action::Requeue {
            task: id.trim().parse().map_err(|_| USAGE_REQUEUE)?,
        });
    }
    if let Some(arguments) = text.strip_prefix("/repair ") {
        let (id, reason) = arguments.split_once(' ').ok_or(USAGE_REPAIR)?;
        return Ok(Action::Repair {
            task: id.parse().map_err(|_| "Invalid task id")?,
            reason: reason.trim().into(),
        });
    }
    if let Some(arguments) = text.strip_prefix("/permit ") {
        let (id, json) = arguments.split_once(' ').ok_or(USAGE_PERMIT)?;
        let task = id.parse().map_err(|_| "Invalid task id")?;
        let policy: crate::tasks::WorkPolicy = serde_json::from_str(json)
            .map_err(|_| "Policy must be JSON with exact files and check argv, e.g. {\"files\":[\"calc.py\"],\"check\":[\"python3\",\"-m\",\"unittest\"]}")?;
        policy.validate()?;
        return Ok(Action::Permit { task, policy });
    }
    if let Some(arguments) = text.strip_prefix("/review ") {
        let task = arguments
            .split(' ')
            .next()
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or("Invalid task id")?;
        let json = arguments
            .split_once(' ')
            .map_or("", |(_, json)| json.trim());
        if json.is_empty() {
            return Err(review_usage(task, 1));
        }
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum ReviewInput {
            Decision(crate::assessment::Decision),
            Assessment(crate::assessment::Assessment),
        }
        if let Err(error) = serde_json::from_str::<serde_json::Value>(json) {
            return Err(format!(
                "Review JSON is not valid JSON (line {} column {})",
                error.line(),
                error.column()
            ));
        }
        match serde_json::from_str::<ReviewInput>(json).map_err(|_| "Review JSON needs outcome (or legacy accept), reason, criteria; limited approval also needs limitations")? {
            ReviewInput::Decision(decision) => {
                decision.validate()?;
                return Ok(if decision.proposes_repair() && decision.failure == Some(crate::assessment::FailureKind::Architecture) { Action::ReviewArchitecture { task, decision } } else if decision.proposes_repair() { Action::ReviewAndRepair { task, decision } } else { Action::Decide { task, decision } });
            }
            ReviewInput::Assessment(assessment) => { assessment.validate()?; return Ok(Action::Assess { task, assessment }); }
        }
    }
    for (prefix, accept) in [("/accept ", true), ("/reject ", false)] {
        if let Some(id) = text.strip_prefix(prefix) {
            return Ok(Action::Review {
                task: id.parse().map_err(|_| "Invalid task id")?,
                accept,
            });
        }
    }
    if let Some(title) = word_arguments(text, "/task") {
        if title.is_empty() {
            return Err(bare_usage("/task").into());
        }
        return Ok(Action::Propose {
            title: title.into(),
            model: model.into(),
            dependencies: vec![],
        });
    }
    if let Some(arguments) = word_arguments(text, "/after") {
        let (ids, title) = arguments
            .split_once(char::is_whitespace)
            .map_or((arguments, ""), |(ids, title)| (ids, title.trim()));
        if ids.is_empty() || title.is_empty() {
            return Err(bare_usage("/after").into());
        }
        let dependencies = ids
            .split(',')
            .map(|id| {
                id.parse::<u64>()
                    .map_err(|_| "Invalid dependency id".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Action::Propose {
            title: title.into(),
            model: model.into(),
            dependencies,
        });
    }
    for (prefix, approve) in [("/approve ", true), ("/cancel-task ", false)] {
        if let Some(id) = text.strip_prefix(prefix) {
            let task = id.trim().parse().map_err(|_| "Use a numeric task id")?;
            return Ok(if approve {
                Action::Approve { task }
            } else {
                Action::Cancel { task }
            });
        }
    }
    if BARE_USAGE.iter().any(|(verb, _)| *verb == text) {
        return Err(bare_usage(text).into());
    }
    Err(unknown_command(text))
}

const USAGE_ASSIGN: &str = "Usage: /assign ID MODEL";
const USAGE_PERMIT: &str =
    "Usage: /permit ID {\"files\":[\"path\"],\"check\":[\"program\",\"arg\"]}";
const USAGE_REPAIR: &str = "Usage: /repair ID reason";
const USAGE_REQUEUE: &str = "Usage: /requeue ID";
const USAGE_RESOLVE_REPAIR: &str = "Usage: /resolve-repair ID";
const USAGE_SCOPE_CONFIRM: &str = "Usage: /scope-confirm DRAFT_REVISION";
const USAGE_ARCHITECT_REVISE: &str = "Usage: /architect-revise ID";

/// Usage for commands that need arguments, shown when typed bare. Each matches
/// `VERB ` with a trailing space above, so the bare word would otherwise reach
/// `unknown_command`.
const BARE_USAGE: &[(&str, &str)] = &[
    ("/task", "Usage: /task DESCRIPTION"),
    ("/after", "Usage: /after IDS DESCRIPTION"),
    ("/assign", USAGE_ASSIGN),
    ("/permit", USAGE_PERMIT),
    ("/repair", USAGE_REPAIR),
    ("/requeue", USAGE_REQUEUE),
    ("/resolve-repair", USAGE_RESOLVE_REPAIR),
    ("/review", "Usage: /review ID JSON"),
    ("/scope-confirm", USAGE_SCOPE_CONFIRM),
    ("/architect-revise", USAGE_ARCHITECT_REVISE),
    ("/retry-command", "Usage: /retry-command SESSION:COMMAND"),
];

fn bare_usage(verb: &str) -> &'static str {
    BARE_USAGE
        .iter()
        .find(|(known, _)| *known == verb)
        .map_or("", |(_, usage)| usage)
}

/// Arguments after `word` in trimmed `text`: empty for the bare word, `None`
/// when `text` is another command that merely starts with the same letters.
fn word_arguments<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(word)?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// Short feedback for a command nobody handles; it names the command, points to
/// F1 and shows the autopilot entry point, and stays within 80 columns.
fn unknown_command(text: &str) -> String {
    let verb = text.split_whitespace().next().unwrap_or(text);
    let mut name: String = verb.chars().take(14).collect();
    if verb.chars().count() > 14 {
        name.push('…');
    }
    format!("Unknown command {name} · F1 lists commands · /go GOAL starts autopilot")
}

#[cfg(test)]
mod unknown_command_tests {
    use super::*;

    #[test]
    fn unknown_command_names_it_and_points_to_f1_and_go() {
        for command in ["/help", "/quit", "/exit", "/reset", "/new"] {
            let message = parse(command, "model").unwrap_err();
            assert!(message.contains(command), "{message}");
            assert!(message.contains("F1"), "{message}");
            assert!(message.contains("/go GOAL"), "{message}");
            assert!(message.chars().count() <= 80, "{message}");
        }
    }

    #[test]
    fn task_and_after_keep_parsing_valid_input_and_name_missing_arguments() {
        let propose = |title: &str, dependencies: Vec<u64>| Action::Propose {
            title: title.into(),
            model: "model".into(),
            dependencies,
        };
        assert_eq!(
            parse("/task  fix it ", "model"),
            Ok(propose("fix it", vec![]))
        );
        assert_eq!(
            parse("/after 1,2 add tests", "model"),
            Ok(propose("add tests", vec![1, 2]))
        );
        assert_eq!(
            parse("/after x add tests", "model").unwrap_err(),
            "Invalid dependency id"
        );
        assert_eq!(
            parse("/tasks", "model").unwrap_err(),
            unknown_command("/tasks")
        );
        assert_eq!(
            parse("/after 1", "model").unwrap_err(),
            bare_usage("/after")
        );
    }

    #[test]
    fn a_very_long_unknown_command_name_stays_short() {
        let message = parse(&format!("/{} arg", "x".repeat(500)), "model").unwrap_err();
        assert!(
            message.contains("F1") && message.contains("/go GOAL"),
            "{message}"
        );
        assert!(message.chars().count() <= 80, "{message}");
    }

    #[test]
    fn review_usage_shows_a_template_with_the_real_criterion_numbers() {
        let message = parse("/review 3", "model").unwrap_err();
        assert!(message.starts_with("Usage: /review 3 {"), "{message}");
        assert_eq!(parse("/review 3 ", "model").unwrap_err(), message);
        assert!(
            !review_usage(3, 0).contains("criterion"),
            "no fake criterion"
        );
        let message = review_usage(3, 2);
        assert!(
            message.contains(
                r#"/review 3 {"outcome":"approved","reason":"…","criteria":[{"criterion":1,"met":true,"note":"…"},{"criterion":2,"met":true,"note":"…"}]}"#
            ),
            "{message}"
        );
    }

    #[test]
    fn review_json_syntax_errors_report_position_apart_from_missing_fields() {
        let syntax = parse("/review 1 {oops", "model").unwrap_err();
        assert_eq!(syntax, "Review JSON is not valid JSON (line 1 column 2)");
        let fields = parse(r#"/review 1 {"accept":true}"#, "model").unwrap_err();
        assert!(fields.starts_with("Review JSON needs outcome"), "{fields}");
        assert!(!fields.contains("not valid JSON"), "{fields}");
    }
}
