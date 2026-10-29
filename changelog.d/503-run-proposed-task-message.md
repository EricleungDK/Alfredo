### Fixed
- `/run` on a proposed task now says "Task #N is proposed; /approve N first" instead of the internal worker-owner rule, and an approved task blocked by an unaccepted dependency is refused with the dependency named, without leaving a stale "start paused" row in the task inspector.
