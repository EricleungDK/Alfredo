### Fixed

- A model reply with no text (an empty `done`, also after tool rounds) now fails the turn
  with `Model returned no text; Ctrl+R retries or send again` instead of showing a blank
  Assistant block, and the empty reply is no longer sent as context. Headless `--chat`
  reports `state` `failed`, `error` set and exit 1 instead of `done` and exit 0.
