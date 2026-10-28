### Security

- Plan validation no longer runs host git on the drawing thread, and its git reads now use the bounded, hardened path with a time limit. A repository whose config includes a named pipe could freeze the terminal until it was killed; now such a repository is treated as unknown and the freeze cannot happen (#519).

### Changed

- Plan validation warnings for a draft appear a moment after the draft, while they are checked in the background.
