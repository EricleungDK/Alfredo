### Fixed

- A task cancelled before it ever ran is no longer sent to `/requeue N` (which the
  store refuses) or `/run N first`: `/run`, `/evidence` and the other not-run refusals now
  say it was cancelled before it ran and name `/task` for new work. A task cancelled
  during a run still gets `/requeue N to approve it again`, and the `/requeue` help says
  it does not apply to a task cancelled before it ran.
- `/run`, `/accept`, `/cancel-task` and `/repair` on a task the store still shows as
  running, with no worker in this terminal (after a crash or another terminal's worker),
  now say `Task #N is running` and name `/recover N` instead of the internal "acquire a
  new worker owner" rule or "no evidence receipt", and no longer leave a stuck
  `start paused` row. The task detail Stage row now prints the real id
  (`/recover 1`), not the literal `/recover ID`.
