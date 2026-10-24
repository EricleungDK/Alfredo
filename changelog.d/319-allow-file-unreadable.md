### Fixed

- A present but unreadable `chat-allow.json` (malformed, over 1 MiB, not a regular file)
  is no longer read as "no rules" and then overwritten by the next `a`. `/allow` shows an
  error naming the file and how to move it aside, `a`, `/allow clear` and
  `/allow clear ID` refuse to write while it is unreadable, and no rule matches (every
  action still asks). A missing or empty file is still no rules.
