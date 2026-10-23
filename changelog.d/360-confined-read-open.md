### Security

- `read_file`, grep and the project notes read (`AGENTS.md`) now open the file without
  following links and check the opened file itself: it must be a regular file inside the
  Coding Workspace and not an excluded path. Before, a file swapped for a symlink (or a
  directory swapped for a link out of the workspace) between the path check and the open
  could put outside text, such as an excluded `.env`, into model context. A FIFO swapped in
  can no longer block a read. Reads are refused where the opened file's location cannot be
  confirmed (no `/proc`, non-Linux Unix) (#360).
