### Fixed

- Chat replies no longer drop `**` that is not bold (`src/**/*.rs`, `2 ** 3`,
  `f(**kw, **o)`, `2**3`, `***x***`): bold needs a non-space after the opener and before
  the closer, and an opener at a word start or after a non-ASCII character.
- A code or fence row cut at the pane width now ends in a visible `…` instead of
  silently losing the rest of the row.
