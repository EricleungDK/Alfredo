### Fixed

- A `/go` (with a goal, bare, or with an open `/plan` draft) while autopilot is already
  running is now refused with a chat row, `✗ Not started · Autopilot is active for “…”;
  /pause or /stop and let workers finish before a new /go`, and a short footer,
  `/go refused: autopilot is active (see the chat rows above)`. The footer used to cut
  the sentence before its fix.
