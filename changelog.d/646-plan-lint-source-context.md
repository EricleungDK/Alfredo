### Fixed

- The planning lint now rejects a task whose approved files each fit the 64 KiB bound but
  together (committed size plus the worker's per-file framing) exceed the 96 KiB worker
  context, with the worker's `Approved files total` message. The failure comes at plan
  time instead of after a worker starts.
