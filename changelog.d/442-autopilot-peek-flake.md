### Fixed

- The autopilot peek test no longer flakes with "Task store is busy" on loaded CI
  runners: its read-only poll now reads the mission state directory without taking the
  task-store lock, and the shared fixture retries a busy refusal for a bounded time
  instead of panicking. Store lock semantics are unchanged.
