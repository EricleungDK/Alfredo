### Fixed

- The approval card (header, args, preview and key line) and the tool-step rows in the
  transcript now show invisible and bidi format characters (such as U+202E, U+200B,
  U+2066, U+E0041) as `<U+XXXX>` instead of drawing them, so a command or path can no
  longer look different from what runs. `propose_plan` and the card share one predicate.
