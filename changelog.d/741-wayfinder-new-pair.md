### Fixed

- A prompt, `/task`, `/plan` or `--go` goal that only mentions "the new app", "new service",
  "new project", "new application", "new repository" or "new product" (for example "fix the new
  app crash") is no longer routed into Wayfinder: chat reaches the model with its tools, `/task`
  proposes the task, and `/go` keeps the chat tools. The pair now starts Wayfinder only after a
  creation verb within four words (build, create, make, start, scaffold, bootstrap, develop,
  generate, set up, spin up, write, init) or at the start of the text. The headless refusal of
  such a goal names the matched phrase and says to rephrase it or use the terminal.
