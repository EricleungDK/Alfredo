### Changed

- Candidate commits now read like history: the subject is the task title (at most 72
  characters, control characters stripped) and the body holds `Alfredo task #N · run ID ·
  files · check` (plus `Repair of #M`). The `alfredo/go-*` integration branch ends in a
  commit whose subject is the goal (first 72 characters).
