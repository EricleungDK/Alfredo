### Added

- Bare `/go` runs a reviewed plan: with a `/plan` draft open it saves the draft
  unchanged and starts autopilot over those tasks, and with none it runs the mission's
  saved, still `proposed` `/plan-save` tasks, approving them under the same unattended
  lint as planner output. No planner request is made, so reviewing a plan first no
  longer costs a second local planning call or orphans the saved tasks. A plan that
  fails the lint is refused with the reason and left untouched. `/go GOAL` with a draft
  open still refuses, now naming bare `/go`; the usage line reads `Usage: /go GOAL, or
  /go to run the open plan`.
