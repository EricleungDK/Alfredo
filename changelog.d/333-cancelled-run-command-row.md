### Fixed

- A `run_command` killed mid-run by Esc, `/stop` or Ctrl+Q now leaves a `⚙ run_command`
  row (`cancelled after Ns · killed; it may have partly run`), and the next request
  carries `run_command <cmd>: cancelled mid-run, effects may be partial`; the turn
  heading reads `· interrupted, applied changes sent as context`. Sandboxed and HOST
  commands alike; the command is never re-run.
