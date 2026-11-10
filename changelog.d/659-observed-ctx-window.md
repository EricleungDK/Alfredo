### Changed

- The chat turn line now shows `ctx used/WINDOW` from the selected model's live
  `context_length` (`GET /api/ps`) when `--num-ctx` is unset, and adds
  "context may be truncated" from 95% of it. A configured `--num-ctx` still wins, and
  `ctx used/default` remains when the server reports no window.
