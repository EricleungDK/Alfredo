### Fixed

- Chat tool results are now budgeted to the room left: under `--num-ctx` a 16 KiB `read_file` no longer ends the turn with `prompt ≈N tokens > --num-ctx W`, and near the 128 KiB conversation limit results are trimmed or replaced by a note and the model gets a final round to answer instead of `Conversation output limit reached`. The `--num-ctx` prompt estimate now counts the tool definitions, and the failures name a way out (`/compact`, `/clear`, Ctrl+N, a larger `--num-ctx`).
