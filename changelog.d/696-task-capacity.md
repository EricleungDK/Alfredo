### Fixed

- At the 256-task limit, `/go`, `/plan` and `--go GOAL --headless` now refuse before any
  planner request and name the limit and the way out (a new mission: `--new-mission NAME`
  or Start New Mission in the mission picker), instead of spending a planning request and
  pausing with a bare "Task capacity reached". Headless exits 2 and leaves no run to
  resume; a plan that does not fit says how many task slots it needs and how many are
  free; a limit reached during a `/go` run ends it as failed rather than paused.
- A one-time notice `Task list 80% full (205/256); start a new mission before it fills`
  appears once the task list reaches 80% of its limit.
