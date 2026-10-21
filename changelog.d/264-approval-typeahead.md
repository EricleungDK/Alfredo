### Fixed

- A `y` or `n` typed into the prompt while a chat approval card is pending no longer
  decides the card ("hey, check the remote first" used to approve a `git_push`). While
  the draft has text, `y`/`n` go into it; with an empty draft they answer only after a
  0.5 s pause with no other key. The prompt title says how to answer when the draft has
  text.
