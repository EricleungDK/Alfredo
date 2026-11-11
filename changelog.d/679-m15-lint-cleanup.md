### Changed

- Internal code-health cleanup, no behaviour change: clippy pedantic warnings fall from 1016 to 974 (the `single_match_else`, `assigning_clones`, `must_use`, cast, `# Errors` and `# Panics` lints), and the chat-health window lookup no longer panics on a poisoned lock.
