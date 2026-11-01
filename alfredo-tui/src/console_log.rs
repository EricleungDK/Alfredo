use super::{PlanOrigin, Session, TaskReceiptRef};

const COMMAND_BUDGET_COUNT: usize = 512;
const COMMAND_BUDGET_BYTES: usize = 512 * 1024;
/// Largest console sequence a saved command or task receipt may carry.
const MAX_CONSOLE_SEQUENCE: u64 = 8192;
/// How many evicted commands' ids and correlations are remembered; older ones are
/// forgotten, and the stores' own duplicate guards remain the backstop.
const RETIRED_COMMANDS: usize = 1024;
const CAPACITY: &str = "Command history capacity reached; start another conversation";
/// Normalized command JSON plus 1 KiB reserved per command for terminal metadata.
fn command_budget_use(commands: &[crate::console_command::ConsoleCommand]) -> usize {
    budget_bytes(
        commands.len(),
        command_sizes(commands)
            .iter()
            .fold(0, |sum, size| sum.saturating_add(*size)),
    )
}
/// The JSON array of `count` commands whose sized lengths add up to `sum`: brackets and
/// one comma between neighbours.
fn budget_bytes(count: usize, sum: usize) -> usize {
    if count == 0 {
        2
    } else {
        sum.saturating_add(count + 1)
    }
}
/// What an evicted command keeps refused: its id first, then each correlation the
/// structure rules keep unique among evictable commands (selection commands are never
/// evicted).
fn retirement_keys(command: &crate::console_command::ConsoleCommand) -> Vec<String> {
    let mut keys = vec![format!("id:{}", command.id)];
    let intent = &command.intent;
    if let Some(request) = intent.scope_request() {
        keys.push(format!("scope:{}", request.correlation));
    }
    if let Some(request) = intent.planner_request() {
        keys.push(format!("planner:{}", request.correlation));
    }
    keys
}

/// Each command's normalized JSON length plus its 1 KiB reservation, serialized once.
fn command_sizes(commands: &[crate::console_command::ConsoleCommand]) -> Vec<usize> {
    commands
        .iter()
        .map(|command| {
            let mut budgeted = command.clone();
            budgeted.state = crate::console_command::CommandState::Pending;
            budgeted.attempt = 1;
            serde_json::to_vec(&budgeted)
                .map_or(usize::MAX, |bytes| bytes.len())
                .saturating_add(1024)
        })
        .collect()
}

impl Session {
    fn next_sequence_value(&self) -> u64 {
        self.task_receipts
            .iter()
            .map(|r| r.sequence)
            .chain(self.commands.iter().map(|c| c.sequence))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
    fn next_console_sequence(&mut self) -> u64 {
        if self.next_sequence_value() > MAX_CONSOLE_SEQUENCE {
            self.renumber_console_sequences();
        }
        self.next_sequence_value()
    }
    /// Evictions leave gaps, so the lifetime counter would otherwise reach the cap with
    /// few commands left. Renumber 1.. in the existing order (parents stay earlier,
    /// ids are intent hashes and unchanged) and move the reader's command anchor along.
    fn renumber_console_sequences(&mut self) {
        let mut old: Vec<u64> = self
            .task_receipts
            .iter()
            .map(|r| r.sequence)
            .chain(self.commands.iter().map(|c| c.sequence))
            .filter(|sequence| *sequence > 0)
            .collect();
        old.sort_unstable();
        let renumbered = |sequence: u64| old.binary_search(&sequence).map_or(0, |at| at as u64 + 1);
        for reference in &mut self.task_receipts {
            reference.sequence = renumbered(reference.sequence);
        }
        for command in &mut self.commands {
            command.sequence = renumbered(command.sequence);
        }
        self.remap_reading_commands(|sequence| Some(renumbered(sequence)));
    }
    /// Move or drop the reader's anchor on a command (`None` drops it).
    fn remap_reading_commands(&self, map: impl Fn(u64) -> Option<u64>) {
        let mut reading = self.reading.get();
        reading.remap_block(|key| match key {
            crate::reading::BlockKey::Command(sequence) => {
                map(sequence).map(crate::reading::BlockKey::Command)
            }
            other => Some(other),
        });
        self.reading.set(reading);
    }
    pub fn commands(&self) -> &[crate::console_command::ConsoleCommand] {
        &self.commands
    }
    pub fn submit_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
    ) -> Result<String, String> {
        self.admit_command(
            text,
            intent,
            true,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    /// Record an autopilot-chosen command like typed input, but keep the reader's
    /// position: autopilot must not pull a user who scrolled up to the bottom.
    pub fn submit_autopilot_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
    ) -> Result<String, String> {
        self.admit_command(
            text,
            intent,
            false,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    /// Append a controller-selected launch at its source without moving the reader
    /// or changing the composer's unfinished work.
    pub fn submit_automatic_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
    ) -> Result<String, String> {
        if !matches!(
            intent,
            crate::command_intent::Intent::DispatchRun { .. }
                | crate::command_intent::Intent::ArchitectDraft { .. }
        ) {
            return Err("Automatic entries require a controller-selected intent".into());
        }
        self.admit_command(
            text,
            intent,
            false,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    /// Keep a delayed scope operation with its exact existing user turn. The
    /// compact command is presentation history, never an extra model message.
    pub fn submit_wayfinder_command(
        &mut self,
        user_message: usize,
        request: crate::understanding::Request,
    ) -> Result<String, String> {
        let after_messages = user_message
            .checked_add(2)
            .ok_or("Wayfinder turn is out of bounds")?;
        if !user_message.is_multiple_of(2)
            || after_messages > self.messages.len()
            || !crate::wayfinder::request_matches_prompt(
                &request,
                &self.messages[user_message].content,
            )
        {
            return Err("Wayfinder request does not match its saved user turn".into());
        }
        let text = match request.action {
            crate::understanding::Action::Enter { .. } => "Enter shared understanding".into(),
            crate::understanding::Action::Draft { .. } => "Save scope draft for review".into(),
            crate::understanding::Action::Confirm { draft_revision } => {
                format!("Confirm shared understanding r{draft_revision}")
            }
        };
        self.admit_command(
            text,
            crate::command_intent::Intent::Wayfinder {
                request,
                user_message,
            },
            false,
            Some(after_messages),
            crate::console_command::CommandState::Pending,
        )
    }
    pub fn submit_selection_command(
        &mut self,
        text: String,
        request: crate::selection_command::Request,
    ) -> Result<String, String> {
        self.admit_command(
            text,
            crate::command_intent::Intent::Selection { request },
            false,
            None,
            crate::console_command::CommandState::Pending,
        )
    }
    pub fn submit_selection_arrival(
        &mut self,
        request: crate::selection_command::Request,
        outcome: crate::selection_command::Outcome,
    ) -> Result<String, String> {
        // One arrival per workspace and mission: a relaunch supersedes the earlier
        // banner instead of piling up. The superseded entry is display-only history
        // and the new record carries the live journal correlation. Restored on
        // failure so a refused admission changes nothing.
        let target = request.choice.target();
        let mission = request.choice.mission.name().to_owned();
        let before = self.commands.clone();
        self.commands.retain(|command| {
            !matches!(&command.intent, crate::command_intent::Intent::SelectionArrival { request }
                if request.choice.target() == target && request.choice.mission.name() == mission)
        });
        let admitted = self.admit_arrival(request, outcome);
        if admitted.is_err() {
            self.commands = before;
        }
        admitted
    }
    fn admit_arrival(
        &mut self,
        request: crate::selection_command::Request,
        outcome: crate::selection_command::Outcome,
    ) -> Result<String, String> {
        self.admit_command(
            format!(
                "Workspace {} · mission {}",
                request.choice.target().display(),
                request.choice.mission.name()
            ),
            crate::command_intent::Intent::SelectionArrival { request },
            false,
            None,
            crate::console_command::CommandState::Selection { outcome },
        )
    }
    /// Admit a command; a refusal leaves commands, task receipts and the reader's
    /// anchor exactly as found, even when the lifetime numbers were renumbered for it.
    fn admit_command(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
        reveal: bool,
        after_messages: Option<usize>,
        state: crate::console_command::CommandState,
    ) -> Result<String, String> {
        let saved = (self.next_sequence_value() > MAX_CONSOLE_SEQUENCE).then(|| {
            (
                self.commands.clone(),
                self.task_receipts.clone(),
                self.reading.get(),
            )
        });
        let admitted = self.admit_command_numbered(text, intent, reveal, after_messages, state);
        if let (Err(_), Some((commands, receipts, reading))) = (&admitted, saved) {
            self.commands = commands;
            self.task_receipts = receipts;
            self.reading.set(reading);
        }
        admitted
    }
    fn admit_command_numbered(
        &mut self,
        text: String,
        intent: crate::command_intent::Intent,
        reveal: bool,
        after_messages: Option<usize>,
        state: crate::console_command::CommandState,
    ) -> Result<String, String> {
        use crate::console_command::ConsoleCommand;
        let command = ConsoleCommand {
            id: ConsoleCommand::identity(&intent),
            sequence: self.next_console_sequence(),
            after_messages: after_messages.unwrap_or(self.messages.len()),
            attempt: 1,
            text,
            intent,
            state,
        };
        if self.commands.iter().any(|old| old.id == command.id) {
            return Err("Retry the original command entry".into());
        }
        if !command.valid(self.messages.len()) {
            return Err("Invalid command intent".into());
        }
        let keys = retirement_keys(&command);
        if self.retired_commands.contains(&keys[0]) {
            return Err("Retry the original command entry".into());
        }
        if keys[1..]
            .iter()
            .any(|key| self.retired_commands.contains(key))
        {
            return Err("Command correlation was already used; submit a new command".into());
        }
        self.commands.push(command);
        // Everything but capacity is judged on the full history, before any eviction, so
        // a duplicate correlation is refused at the budget exactly as under it.
        if !self.valid_command_structure(17) {
            let refusal = self.structure_refusal();
            self.commands.pop();
            return Err(refusal.into());
        }
        // Only a full budget evicts, and only settled commands; a refusal drops nothing.
        if !self.commands_within_budget() && !self.evict_settled_commands() {
            self.commands.pop();
            return Err(CAPACITY.into());
        }
        if reveal {
            self.scroll.set(0);
            self.reading.set(Default::default());
        }
        Ok(self.commands.last().unwrap().id.clone())
    }
    pub fn set_command_state(
        &mut self,
        id: &str,
        state: crate::console_command::CommandState,
    ) -> bool {
        let Some(command) = self.commands.iter_mut().find(|command| command.id == id) else {
            return false;
        };
        command.state = state;
        true
    }
    pub fn retry_command(&mut self, id: &str) -> Result<(), String> {
        let command = self
            .commands
            .iter_mut()
            .find(|command| command.id == id)
            .ok_or("Command origin unavailable")?;
        if command.intent.selection_request().is_some() {
            return Err(
                "Selection requires journal reconciliation; use /workspace for a new choice".into(),
            );
        }
        if matches!(
            command.intent,
            crate::command_intent::Intent::DispatchRun { .. }
        ) {
            return Err("Automatic launch is historical; submit /run for an explicit retry".into());
        }
        if matches!(
            command.intent,
            crate::command_intent::Intent::ArchitectDraft { .. }
        ) {
            return Err("Automatic Architect draft is historical; submit /architect-revise for an explicit retry".into());
        }
        if matches!(
            command.state,
            crate::console_command::CommandState::Planner { .. }
                | crate::console_command::CommandState::Control { .. }
        ) {
            return Err("Command has a recorded local outcome; submit a new command".into());
        }
        command.attempt = command
            .attempt
            .checked_add(1)
            .filter(|n| *n < u64::MAX)
            .ok_or("Retry limit reached")?;
        command.state = crate::console_command::CommandState::Pending;
        Ok(())
    }
    pub(crate) fn restore_commands(&mut self) {
        use crate::console_command::CommandState;
        for command in &mut self.commands {
            if matches!(
                command.state,
                CommandState::Pending | CommandState::Submitted
            ) {
                command.state = CommandState::Unknown {
                    reason: if command.intent.selection_request().is_some() {
                        "Interrupted selection; inspect journal and use /workspace for a new choice"
                    } else {
                        "Interrupted command; reconcile receipts before explicit retry"
                    }
                    .into(),
                };
            }
        }
    }
    pub(crate) fn valid_commands(&self, schema: u32) -> bool {
        self.valid_command_structure(schema)
            && self.commands_within_budget()
            && self.retired_commands.len() <= RETIRED_COMMANDS
            && self.retired_commands.iter().all(|key| {
                !key.is_empty() && key.len() <= 512 && !key.chars().any(char::is_control)
            })
    }
    /// Why the history just admitted to (its last command) breaks a structure rule.
    fn structure_refusal(&self) -> &'static str {
        let Some((last, earlier)) = self.commands.split_last() else {
            return "Invalid command history";
        };
        let key_of = |command: &crate::console_command::ConsoleCommand| {
            retirement_keys(command).split_off(1)
        };
        let keys = key_of(last);
        if earlier
            .iter()
            .any(|other| key_of(other).iter().any(|k| keys.contains(k)))
        {
            "Command correlation is already in use; submit a new command"
        } else if matches!(
            last.intent,
            crate::command_intent::Intent::ArchitectDraft { .. }
                | crate::command_intent::Intent::DispatchRun { .. }
        ) {
            "Command parent is missing or not yet settled; nothing was recorded"
        } else {
            "Invalid command history; nothing was recorded"
        }
    }
    /// Every command rule except the size budget.
    fn valid_command_structure(&self, schema: u32) -> bool {
        use std::collections::BTreeSet;
        if (schema < 10 && !self.commands.is_empty())
            || (schema < 12
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::Planner { .. }
                    ) || matches!(
                        command.state,
                        crate::console_command::CommandState::Planner { .. }
                    )
                }))
            || (schema < 13
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::Control { .. }
                    ) || matches!(
                        command.state,
                        crate::console_command::CommandState::Control { .. }
                    )
                }))
            || (schema < 14
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::DispatchRun { .. }
                    )
                }))
            || (schema < 15
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::ArchitectDraft { .. }
                    )
                }))
            || (schema < 16
                && self.commands.iter().any(|command| {
                    matches!(
                        command.intent,
                        crate::command_intent::Intent::Wayfinder { .. }
                    )
                }))
            || (schema < 17
                && self.commands.iter().any(|command| {
                    command.intent.selection_request().is_some()
                        || matches!(
                            command.state,
                            crate::console_command::CommandState::Selection { .. }
                        )
                }))
            || self
                .commands
                .iter()
                .any(|c| !c.valid(self.messages.len()) || c.sequence > MAX_CONSOLE_SEQUENCE)
            || self
                .task_receipts
                .iter()
                .any(|reference| reference.sequence > MAX_CONSOLE_SEQUENCE)
        {
            return false;
        }
        let mut ids = BTreeSet::new();
        let mut planner_ids = BTreeSet::new();
        let mut scope_ids = BTreeSet::new();
        let mut selection_ids = BTreeSet::new();
        let mut wayfinder_turns = BTreeSet::new();
        let mut sequences = BTreeSet::new();
        for reference in &self.task_receipts {
            if reference.sequence > 0 && !sequences.insert(reference.sequence) {
                return false;
            }
        }
        for command in &self.commands {
            if let Some(request) = command.intent.selection_request() {
                if !selection_ids.insert(&request.correlation) {
                    return false;
                }
            }
            if let Some(request) = command.intent.scope_request() {
                if !scope_ids.insert(&request.correlation) {
                    return false;
                }
            }
            if let crate::command_intent::Intent::Wayfinder {
                request,
                user_message,
            } = &command.intent
            {
                if !user_message.is_multiple_of(2)
                    || user_message.checked_add(2) != Some(command.after_messages)
                    || self.messages.get(*user_message).is_none_or(|message| {
                        message.role != "user"
                            || !crate::wayfinder::request_matches_prompt(request, &message.content)
                    })
                    || self
                        .messages
                        .get(user_message.saturating_add(1))
                        .is_none_or(|message| message.role != "assistant")
                    || !wayfinder_turns.insert(*user_message)
                {
                    return false;
                }
            }
            if let Some(request) = command.intent.planner_request() {
                if !planner_ids.insert(&request.correlation) {
                    return false;
                }
            }
            if let crate::command_intent::Intent::ArchitectDraft { request } = &command.intent {
                if !self.commands.iter().any(|parent| {
                    parent.sequence < command.sequence
                        && matches!(&parent.intent, crate::command_intent::Intent::Task { request: source } if source == &request.source)
                }) {
                    return false;
                }
            }
            if let crate::command_intent::Intent::DispatchRun { request } = &command.intent {
                let parent = self.commands.iter().any(|parent| {
                    parent.sequence < command.sequence
                        && matches!(&parent.intent, crate::command_intent::Intent::Control { request: source } if source == &request.source)
                        && matches!(parent.state, crate::console_command::CommandState::Control {
                            outcome: crate::control_command::Outcome::DispatchChanged { enabled: true }
                        })
                });
                if !parent {
                    return false;
                }
            }
            if !ids.insert(&command.id) || !sequences.insert(command.sequence) {
                return false;
            }
        }
        if !self
            .commands
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
        {
            return false;
        }
        let chronological: Vec<_> = self
            .commands
            .iter()
            .filter(|command| {
                schema < 16
                    || !matches!(
                        command.intent,
                        crate::command_intent::Intent::Wayfinder { .. }
                    )
            })
            .collect();
        if !chronological
            .windows(2)
            .all(|pair| pair[0].after_messages <= pair[1].after_messages)
        {
            return false;
        }
        if !self.task_receipts.windows(2).all(|pair| {
            pair[1].sequence > pair[0].sequence || pair[0].sequence == 0 && pair[1].sequence == 0
        }) {
            return false;
        }
        true
    }
    /// The command-history budget alone: 512 commands and 512 KiB, counting terminal
    /// metadata once against an immutable normalized base.
    pub(crate) fn commands_within_budget(&self) -> bool {
        command_budget_use(&self.commands) <= COMMAND_BUDGET_BYTES
            && self.commands.len() <= COMMAND_BUDGET_COUNT
    }
    /// Set when the history is 80% full: the next commands will drop the oldest settled ones.
    pub fn command_history_warning(&self) -> Option<String> {
        (self.commands.len() * 5 >= COMMAND_BUDGET_COUNT * 4
            || command_budget_use(&self.commands) * 5 >= COMMAND_BUDGET_BYTES * 4)
            .then(|| "Command history 80% full; /clear starts a fresh one".into())
    }
    /// Drop the fewest oldest settled commands that bring the history within budget, and
    /// say whether that was possible. Live (Pending, Submitted), selection, wayfinder,
    /// generated-plan and still-needed parent commands stay, so a restored Unknown or
    /// Refused entry is the usual victim. The command just admitted (last) stays too.
    /// Sizes are measured once per command, then the victims are found in one scan, so the
    /// cost does not grow with the number evicted.
    fn evict_settled_commands(&mut self) -> bool {
        let sizes = command_sizes(&self.commands);
        let (mut left, mut sum) = (
            self.commands.len(),
            sizes.iter().fold(0usize, |a, b| a.saturating_add(*b)),
        );
        let latest_dispatch = self.commands.iter().rposition(|other| {
            matches!(
                other.state,
                crate::console_command::CommandState::Control {
                    outcome: crate::control_command::Outcome::DispatchChanged { .. }
                }
            )
        });
        let mut dropped = Vec::new();
        for (index, size) in sizes
            .iter()
            .enumerate()
            .take(self.commands.len().saturating_sub(1))
        {
            if left <= COMMAND_BUDGET_COUNT && budget_bytes(left, sum) <= COMMAND_BUDGET_BYTES {
                break;
            }
            if self.evictable(index, latest_dispatch) {
                dropped.push(index);
                left -= 1;
                sum = sum.saturating_sub(*size);
            }
        }
        if left > COMMAND_BUDGET_COUNT || budget_bytes(left, sum) > COMMAND_BUDGET_BYTES {
            return false;
        }
        let kept: Vec<_> = (0..self.commands.len())
            .filter(|index| dropped.binary_search(index).is_err())
            .map(|index| self.commands[index].clone())
            .collect();
        let sequences: Vec<u64> = dropped
            .iter()
            .map(|index| self.commands[*index].sequence)
            .collect();
        let before = std::mem::replace(&mut self.commands, kept);
        if !self.commands_within_budget() {
            self.commands = before;
            return false;
        }
        self.retired_commands.extend(
            dropped
                .iter()
                .flat_map(|index| retirement_keys(&before[*index])),
        );
        let excess = self.retired_commands.len().saturating_sub(RETIRED_COMMANDS);
        self.retired_commands.drain(..excess);
        // A reader anchored to an evicted command must not keep a dangling anchor.
        self.remap_reading_commands(|sequence| {
            (!sequences.contains(&sequence)).then_some(sequence)
        });
        true
    }
    /// Name the pending plan draft's origin request (or none once the draft is gone), so
    /// command-history eviction keeps that command in any state: the draft is saved with
    /// it, and a save without it is refused (issue #475).
    pub fn protect_plan_origin(&mut self, origin: Option<&crate::planner_command::Request>) {
        if self.plan_origin.0.as_ref() != origin {
            self.plan_origin = PlanOrigin(origin.cloned());
        }
    }
    fn evictable(&self, index: usize, latest_dispatch: Option<usize>) -> bool {
        use crate::{
            command_intent::Intent, console_command::CommandState, control_command::Outcome,
            planner_command::Outcome as Planned,
        };
        let command = &self.commands[index];
        let enabling = latest_dispatch == Some(index)
            && matches!(
                command.state,
                CommandState::Control {
                    outcome: Outcome::DispatchChanged { enabled: true }
                }
            );
        let named_by_child =
            self.commands
                .iter()
                .any(|child| match (&child.intent, &command.intent) {
                    (Intent::ArchitectDraft { request }, Intent::Task { request: source }) => {
                        &request.source == source
                    }
                    (Intent::DispatchRun { request }, Intent::Control { request: source }) => {
                        &request.source == source
                    }
                    _ => false,
                });
        !(matches!(
            command.state,
            CommandState::Pending
                | CommandState::Submitted
                | CommandState::Selection { .. }
                | CommandState::Planner {
                    outcome: Planned::Generated { .. }
                }
        ) || command.intent.selection_request().is_some()
            || matches!(command.intent, Intent::Wayfinder { .. })
            || enabling
            || named_by_child
            || self.plan_origin.0.is_some()
                && command.intent.planner_request() == self.plan_origin.0.as_ref())
    }
    pub fn task_receipts(&self) -> &[TaskReceiptRef] {
        &self.task_receipts
    }
    pub fn observe_task_receipt(&mut self, mut reference: TaskReceiptRef) -> bool {
        if self.status.active()
            || self.task_receipts.len() >= 4096
            || !self.valid_task_reference(&reference)
            || self.task_receipts.iter().any(|old| {
                old.revision >= reference.revision
                    || old.correlation == reference.correlation
                    || old.after_messages > reference.after_messages
            })
        {
            return false;
        }
        reference.sequence = self.next_console_sequence();
        if reference.sequence > MAX_CONSOLE_SEQUENCE {
            return false;
        }
        self.task_receipts.push(reference);
        true
    }
    fn valid_task_reference(&self, reference: &TaskReceiptRef) -> bool {
        reference.after_messages <= self.messages.len()
            && reference.after_messages.is_multiple_of(2)
            && (1..=4096).contains(&reference.revision)
            && (1..=256).contains(&reference.task)
            && !reference.correlation.trim().is_empty()
            && reference.correlation.len() <= 160
            && !reference.correlation.chars().any(char::is_control)
    }
    pub(crate) fn valid_task_receipts(&self, schema: u32) -> bool {
        (schema >= 8 || self.task_receipts.is_empty())
            && (schema >= 10
                || self
                    .task_receipts
                    .iter()
                    .all(|reference| reference.sequence == 0))
            && self.task_receipts.len() <= 4096
            && self
                .task_receipts
                .iter()
                .all(|reference| self.valid_task_reference(reference))
            && self.task_receipts.windows(2).all(|pair| {
                pair[0].revision < pair[1].revision
                    && pair[0].after_messages <= pair[1].after_messages
            })
            && self
                .task_receipts
                .iter()
                .map(|reference| &reference.correlation)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == self.task_receipts.len()
    }
}
