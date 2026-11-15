### Security

- A task's policy files may no longer name a secret or generated path (such as `.env`,
  a private key, `target/` or `node_modules/`), so `/permit` and plans refuse them. A
  task saved with one fails before any model call instead of sending the file's content.
- A policy file with an invisible or bidirectional character is refused and the planner
  names the code point, as it already did for prompts and check arguments; the task
  inspector shows such a character visibly in the Files row.
