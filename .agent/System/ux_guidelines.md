# UX Guidelines — Albert Command Deck

**Last Updated:** 2026-06-21

## Design Philosophy

The production direction is the approved industrial Command Deck: a persistent Agent Console left lane and a focused Operations Workspace. The interface is a projection of acknowledged Orchestrator state. Loading or failed connections must never resemble accepted mission state.

## Visual System

- Near-black operational surfaces with restrained lime for launch eligibility/healthy state and cyan for connection/scope information.
- Rajdhani supplies compact mission headings; IBM Plex Mono carries operational detail.
- Borders communicate structure, not decoration. Uppercase text is limited to compact metadata.
- Issue Graph rows prioritize identifier, eligibility, and blocker state over dashboard decoration.

## Interaction Rules

- Conversation Scope stays visible in the Agent Console.
- Scope changes require an explicit selection and Apply action. Merely navigating, selecting an Issue Slice, reconnecting, or changing Active Mission must not retarget the current draft.
- A submitted message carries the exact acknowledged scope displayed at send time; rejected submissions preserve the draft.
- Agent Console history and unfinished drafts stay continuous while the Operations Workspace projection changes.
- Every message or narration record displays its source and outcome. Model commentary must never be styled or described as an acknowledgement.
- Context Inspector shows the bounded character count and each source's category, label, content, and disposition while the full Agent Console remains independently scrollable.
- Governed Workspace Session and Shared Context sources are visibly required and expose no pin/exclude controls.
- Eligible source changes show Pending without optimistic projection changes; only acknowledgement plus authoritative reload may show pinned, excluded, or included state.
- Restored Operations Workspace selection comes from acknowledged persisted preferences.
- Disabled controls must not imply an action was accepted.
- Retry replaces the error state with loading, then renders accepted state only after a successful snapshot.

## Accessibility

- Agent Console is a named region and Operations Workspace is the named main landmark.
- Connection progress uses a polite status; startup and persistence failures use alerts.
- Keyboard focus uses a visible cyan outline.
- Motion is removed under `prefers-reduced-motion`.

## Empty / Loading / Error States

- Loading says that Albert is being contacted and that an authoritative snapshot is pending.
- Empty workspace is a valid acknowledged snapshot with zero Issue Slices.
- Backend startup and persistence-read failures show diagnostic detail and a retry action when recoverable.
- Failure states render no Agent Console mission data and no accepted mission heading.

## Animation & Transitions

Use one restrained boot signal and short Issue Graph entry transitions. Motion is operational feedback, not ambient decoration.
