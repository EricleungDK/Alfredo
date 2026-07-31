# Prototype verdict

Status: **pending live Mission Commander review**

Question answered by this artifact:

> Does the proposed supervision ledger keep canonical state, advisory facts,
> durable attention, cursor progress, and idempotent effects separate enough to
> survive the required fault cuts without noisy healthy polling?

Record the final answer here before deleting or absorbing the prototype.

## Review prompts

1. Should attention and cursor persist in one atomic ledger commit, or is the
   visible outbox-first fallback worth supporting?
2. After exact owner, process-group, operation, worktree, and result-absence
   proof, should same-session infrastructure recovery run automatically, or
   remain visible for a Mission Commander action?
3. At the provisional 15-second cadence, do stale after roughly 45 seconds and
   unavailable after roughly 30 seconds feel too eager, too slow, or
   appropriately adjustable?
4. Should repeated sources merge into one semantic incident as shown, and
   should resolved/superseded attention remain inspectable with its receipt?
5. Is the single derived Mission Work line clear enough, or does operational
   attention need different grouping within the already-approved Focus desk?
6. Is a persisted but unreconciled result correctly more important than a dead
   owner, preventing a duplicate rerun until result validation finishes?
7. Should an immutable runner operation identity join PID/start/group identity
   as a required canonical recovery boundary?
