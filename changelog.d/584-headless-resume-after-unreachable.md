### Added

- `--resume --headless` now also continues a run that stopped because the model server was
  unreachable: it queues the same owner retry as `/repair ID retry` and carries on without
  planning again, instead of exiting 2 "nothing to resume". The family gets one repair beyond
  `--max-repairs`; runs that failed for any other cause still exit 2.

### Fixed

- A headless `--go` stopped by an unreachable model server now names the stopped task in
  `failed[0].id` (it was `null`) and prints a `next:` line on stderr pointing at
  `--resume --headless`.
