### Changed

- `/stop` while autopilot is still planning now cancels the planner request, so no
  plan draft appears later and a new `/go` starts; `/resume` plans again. If the
  stopped autopilot's own draft is already open, `/go` says to `/resume` it or
  `/plan-cancel` it. A draft from your own `/plan` is never discarded by `/go`.
