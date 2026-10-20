### Fixed

- Concurrent writers to the understanding state no longer fail with "Understanding state busy" on slow disks: the scope lock now waits up to 3 seconds (with backoff) instead of 250 ms before asking you to retry.
