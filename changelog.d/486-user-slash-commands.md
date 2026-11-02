### Added

- User-defined slash commands: `NAME.md` in `<State Directory>/commands/` runs as
  `/NAME args`, sending its body as your chat message with `$ARGUMENTS` replaced. Commands
  are listed in F1 and the `/` type-ahead as user commands, load only from the State
  Directory (never the repository), refuse built-in names, symlinks and files over
  64 KiB, and every tool call they cause still shows an approval card.
