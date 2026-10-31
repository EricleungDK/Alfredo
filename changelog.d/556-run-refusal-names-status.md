### Fixed

- `/run` on an accepted, failed, rejected, review-ready, cancelled or needs-human-review
  task now says `Task #N is <status>` and names the next command (`/repair`, `/accept` or
  `/review`, `/requeue`) instead of the internal "acquire a new worker owner" rule, and
  no longer leaves a stuck `start paused` row in the task inspector.
