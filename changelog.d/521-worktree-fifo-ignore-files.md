### Security

- A FIFO, socket or device named `.gitignore`, `.gitattributes` or `.gitmodules` in the
  work tree no longer stalls host git for its full time limit. Git status, working-tree
  diffs, `@path` attachments and `@` completion, and the uncommitted-changes check at `/go`
  start, refuse at once and name the entry instead. `@` completion makes no git call for two
  seconds after git gives up on a listing, so typing `@` does not freeze the screen.
