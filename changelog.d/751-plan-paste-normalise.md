### Fixed

- A pasted multi-line `/plan` or `/plan-revise` request is no longer refused with
  `Invalid planner command bounds or source binding`: each run of line breaks becomes one
  space, as for `/go`. A request over 8192 bytes names its length and the limit, and
  `/plan` followed directly by a line break is the same as `/plan` followed by a space.
- Tabs, vertical tabs, form feeds and U+0085 in a goal (for example tab-indented list items
  in `--go "$(cat task.md)"`) now join like line breaks instead of being refused as control
  characters. ESC, NUL and DEL are still refused.
