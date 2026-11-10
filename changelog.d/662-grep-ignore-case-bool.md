### Fixed

- Chat `grep` now refuses a non-boolean `ignore_case` (`"true"`, `1`) with `error: argument
  ignore_case must be true or false`, like `run_command`'s `escalate`, instead of silently
  searching case-sensitively and answering `no matches` for text that exists.
