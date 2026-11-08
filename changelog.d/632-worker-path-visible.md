### Security

- A worker's FILE or EDIT path now shows its bidi overrides and isolates, zero-width
  and tag characters, and U+2028/U+2029 as `<U+XXXX>` in the `▸ path` heading, in task
  detail and the agent view. Before, these characters reached the terminal raw, so a path
  that contained a raw U+202E could draw as a different file name, such as `src/atxt.rs`
  for `src/a` + U+202E + `sr.txt`. The path that Alfredo reads and applies is unchanged.
