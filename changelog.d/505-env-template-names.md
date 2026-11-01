### Changed

- The chat tools, `@path` mentions and planner context can read the committed env
  templates `.env.example`, `.env.sample` and `.env.template` in any directory. Every other
  `.env*` name (`.env`, `.env.local`, `.env.example.local`, `prod.env`, `.envrc`) stays
  refused, and git-ignored files stay hidden (#505).
