# Shell Terminal Left-Lane Design

## Purpose

Complete Command Deck Issue 13 by adding Shell Terminal as a distinct left-lane mode. The design extends the approved Variant A prototype without broadly redesigning the production interface.

The Mission Commander can run governed commands, resolve required approvals, inspect actionable failures, and create bounded Additional Path Grants. Terminal bytes remain transient UI response state and never become Agent Console history, canonical Workspace Session state, or Activity Journal entries.

## Prior Decisions

- Variant A, Command Deck, remains the approved interaction direction.
- The left lane switches between Agent Console and Shell Terminal; Shell Terminal is not an Operations Workspace view.
- Agent Console is always the startup mode. Mode selection is local UI state and is not restored after application restart.
- Switching modes preserves each mode's independent local draft and visible history for the current application session.
- The existing industrial Command Deck visual language remains: dark high-contrast surfaces, restrained lime and cyan semantic accents, dense precise information, and minimal decorative framing.

The prior-art prototype is `@visualization/albert-mission-control/variant-a.html`. It defines interaction responsibilities, not production HTML, CSS, or in-memory state architecture.

## Left-Lane Structure

The existing left lane gains an accessible two-option mode switch in its header:

- **Agent Console** renders the current continuous conversation, Conversation Scope controls, Context Inspector material, and message composer unchanged.
- **Shell Terminal** replaces the entire left-lane body with terminal-specific controls and output.

Only one mode is visible at a time. Shared parent state retains the inactive mode's draft and session transcript, while its panel is absent from the rendered document and accessibility tree. Switching modes never copies content between the modes or changes accepted Orchestrator state.

Conversation Scope belongs only to Agent Console. Shell Terminal instead displays an execution-boundary summary containing:

- working directory;
- requester;
- requested filesystem paths;
- read or write access level.

This avoids implying that Conversation Scope grants command or filesystem authority.

## Shell Terminal State

React owns only transient presentation state:

- selected left-lane mode;
- command draft;
- working-directory, requested-path, requester, and access-level form values;
- immediate command results, including stdout and stderr;
- pending request indicators and actionable transport failures;
- Additional Path Grant form values.

The Orchestrator owns command classification, command status, decisions, and grants. React loads the authoritative `ShellTerminalProjection` when Terminal mode is first opened and reloads it after successful submit, decision, or grant actions.

Persisted command metadata is rendered from the projection. Immediate stdout and stderr are attached only to the current-session terminal transcript. Reloading or restarting restores metadata without reconstructing terminal bytes.

## Command Submission and Results

The command composer requires command text and working directory. Working directory initially uses the canonical Workspace Session path. Requested paths use a one-path-per-line field that is normalized into the typed request list. The composer also submits the fixed Mission Commander requester and explicit access level through the existing typed client transport.

Observable outcomes are:

- **auto-allowed:** execution returns immediately and the transcript shows exit status plus stdout or stderr;
- **frontier-approvable:** the command appears as pending and identifies Frontier Model as the required approver;
- **human-required:** the command appears as pending and identifies Mission Commander as the required approver;
- **failed:** the transcript shows a non-success exit state and stderr without presenting false success;
- **rejected before persistence:** the form remains intact and displays the actionable backend explanation.

Command text is displayed as text, never interpreted as markup. Submit controls are disabled while the request is pending, but mode switching remains available.

## Approval and Denial

Pending command records expose only valid controls for the current user-facing boundary:

- Mission Commander approval is available for `human-required` commands.
- Frontier approval state is visible for `frontier-approvable` commands; the desktop uses the typed actor boundary and does not relabel a Mission Commander action as Frontier approval.
- Mission Commander denial is available for any pending command and requires a non-empty reason.

Successful decisions reload authoritative metadata and append any immediate execution bytes only to the local terminal transcript. Rejected decisions leave the pending record visible with the backend explanation.

## Additional Path Grants

Terminal mode includes a compact Additional Path Grant form with:

- absolute path;
- read or write access level;
- positive duration in seconds;
- fixed Mission Commander requester attribution.

Created grants are reloaded from the projection and display path, access level, grantor, grant time, and expiry time. Expired grants remain visible as expired history but cannot authorize command execution. The UI offers no edit, renew, broaden, or agent/skill requester controls; a new bounded grant is the only supported follow-up action.

## Errors and Empty States

- Projection load failure shows an actionable Terminal-specific failure and does not synthesize command or grant state.
- Submit, decision, and grant failures remain associated with the relevant form or record.
- Empty command history explains that output is local to this session and persisted metadata will appear after submission.
- Empty grant history explains that workspace-contained commands do not need an Additional Path Grant.
- Offline or unavailable client methods disable mutation controls without hiding already loaded metadata.

## Accessibility and Responsive Behavior

- Agent Console and Shell Terminal are separately named regions.
- The mode control is a two-item tab list. Each tab references its named panel, supports standard arrow-key selection, exposes the selected state, and has visible focus.
- Pending approvals, failures, and expiry are expressed in text, not color alone.
- Command output uses a labelled live region only for completed requests; historical metadata is not repeatedly announced.
- At constrained widths the left lane remains before the Operations Workspace in document order. Terminal forms stack vertically, while pending governance and errors remain reachable without horizontal scrolling.
- Motion is limited to existing purposeful transitions and respects reduced-motion preferences.

## Public-Behavior TDD Plan

Implementation proceeds one observable behavior at a time:

1. Switching to Shell Terminal replaces Agent Console while preserving the untouched Agent Console draft and history when switching back.
2. Opening Terminal loads authoritative metadata and keeps restored records free of stdout and stderr.
3. Auto-allowed submission renders immediate output locally and reloads persisted metadata.
4. Pending human approval exposes approve and reason-required deny controls, with acknowledged and rejected outcomes.
5. Frontier-approvable commands identify the Frontier boundary without offering a falsely attributed Mission Commander approval.
6. Additional Path Grant creation displays bounded access and expiry; no mutation or self-expansion controls exist.
7. Backend path, expiry, denial, and execution failures remain actionable and never appear successful.
8. Constrained and keyboard interaction tests prove both left-lane modes and pending governance remain reachable.

Tests use the public React interaction seam with a contract-faithful `WorkspaceClient`. They assert user-visible behavior and submitted typed requests, not component internals or prototype markup.

## Completion Boundary

Issue 13 is complete only when every ticket acceptance criterion is covered by current tests, the full Python/frontend/typecheck/Rust gates pass, the ticket status and progress documentation are updated, and the architecture records the transient-terminal-byte boundary. Broader accessibility validation and human hierarchy confirmation remain Issue 14 work.
