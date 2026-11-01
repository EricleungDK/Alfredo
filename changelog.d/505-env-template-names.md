### Changed

- The read tools, `@path` mentions and planner context can read `.env.example`,
  `.env.sample` and `.env.template` in any directory. Writes, edits, undo and commits
  still refuse them, every other `.env*` name (`.env`, `.env.local`, `.env.example.local`,
  `prod.env`, `.envrc`) stays refused, and discovery still hides git-ignored files (#505).
