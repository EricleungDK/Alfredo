### Fixed

- Chat git tools now refuse a FIFO, device or link in `HEAD`, `config`, `commondir`, a
  `.git` file or any `objects/info/alternates` (including a chained store's) before git
  runs. Newer git (2.56) opens the alternates file while it locates the repository, so the
  check that came after it let a FIFO stall the call to its 15 s limit; the test
  `an_alternates_fifo_refuses_promptly_and_never_hangs` failed intermittently in CI (#458).
