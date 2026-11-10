### Added

- A chat reply that is only a tool call written as text (a JSON object with a registered
  tool `name` and `arguments`, or `<tool_call>` blocks) now shows `The model wrote a tool
  call as text; nothing ran. Ctrl+R retries.` Nothing is run from model text. Headless
  `--chat` reports it as `notice` in its JSON result; the exit code is unchanged.
