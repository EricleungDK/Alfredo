### Fixed

- `--doctor`, `/go`, headless `--go` and `/run` now check that bubblewrap can actually
  create a sandbox (a no-op probe, once per process outside `--doctor`) and refuse
  before any model call when the host denies it (for example Ubuntu 24.04 user-namespace
  restrictions), quoting bwrap's error. The "sudo apt install bubblewrap" hint now
  appears only when bubblewrap is missing, not when it is installed but denied.
