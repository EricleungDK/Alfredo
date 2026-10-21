### Fixed

- Esc, `/stop`, a chat switch and Ctrl+R now kill a running `run_command` (sandboxed or
  HOST, whole process group, within about a second) instead of letting it finish after the
  turn says "cancelled". Ctrl+Q and every other exit stop a running host command before
  Alfredo exits. The approval log records `cancelled`, not `applied`, for such a run, and
  the chat status reads `running run_command · Ns · Esc stops` while it runs.
