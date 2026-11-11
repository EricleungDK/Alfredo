### Fixed

- A command or `/go` check that prints a non-UTF-8 byte (Latin-1 text, a stray binary
  byte) is no longer reported as `exit code 125` with a false "output passed ... the
  process tree was killed" and no output. The bytes are shown as U+FFFD and the true
  status and exit code are kept (#678).
