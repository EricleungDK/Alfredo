### Fixed

- A sandbox probe that fails once is retried on the next `/go` instead of staying
  refused for the rest of the process; only a successful probe is cached.
