### Fixed

- A goal whose planner prompt is larger than `--num-ctx` now stops `/go` and
  `--go --headless` at once with the provider's reason (`raise --num-ctx (≈N tokens
  needed)`) instead of retrying the planner three times and reporting `Planning failed 3
  times`.
