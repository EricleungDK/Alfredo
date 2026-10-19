### Changed
- Eval: `config` now records the binary path and cargo profile, a debug build warns that M11/M16 are not comparable with the release baselines, M16 `binary_kb` reports the size of the binary under test, and M3-M6 carry `secondary.per_task` so one regressing task is no longer hidden by the global median.
