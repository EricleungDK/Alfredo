### Fixed

- A chat tool call with a long non-ASCII name (for example 40 CJK characters) no longer
  ends the turn with "Conversation output limit reached"; the name is cut to 64 bytes and
  the model gets the normal `unknown tool` result.
