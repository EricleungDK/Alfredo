### Added

- Headless `--go` and `--chat` JSON results gain `usage: {prompt_tokens, completion_tokens,
  model_seconds}` so CI and eval scripts can budget and compare runs. It sums the server's
  reported counts and `total_duration` over the planner, worker and repair calls (`--go`) or
  every round of the turn (`--chat`); a field the server never reported is `null`.
