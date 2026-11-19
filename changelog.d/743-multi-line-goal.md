### Fixed

- A goal pasted over several lines, or passed to `--go` with line breaks, is no longer
  refused by `/go`, `/plan`, `/task`, `/after` and headless `--go`: each run of line
  breaks becomes one space. Other control characters are still refused, and the
  refusal now says line breaks are fine but other control characters are not; a goal
  over its length limit names the length and the limit.
