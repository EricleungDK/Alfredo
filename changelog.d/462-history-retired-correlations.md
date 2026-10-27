### Fixed

- Command history: a correlation or intent id whose command was evicted is still refused
  (the last 1024 are kept in the saved conversation, which older files simply lack); a
  refusal for a repeated correlation or a missing parent now says so instead of `Command
  history capacity reached`; a saved plan draft whose origin command is neither
  generated, live nor interrupted is refused at load and save, since eviction could
  otherwise drop it; the autopilot footer warning at 80% full now joins the existing
  notice like the `/go` one does.
