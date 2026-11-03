### Changed

- A sandboxed `run_command` can now read `.env.example`, `.env.sample` and `.env.template`
  (the names `read_file` already opens): they are bound read-only instead of masked, so
  `cat` and `cp .env.example .env` work while writes, renames and deletes of the template
  fail. They do not count toward the 128-mask limit. `.env` and every other `.env*` name stay
  masked.

### Fixed

- A failing sandboxed run that hides secret files no longer tells the model both to
  retry with `escalate: true` and not to retry on the host, and the hidden-files note now says a
  hidden directory looks empty (`No such file or directory`) rather than `Permission denied`.
