### Added

- For large, multi-file or risky requests, chat can propose `/plan` or `/go` instead of
  editing. The turn ends and your prompt holds `/plan <goal>` (or `/go <goal>`): edit it
  and press Enter to start, or clear it to drop it. Nothing starts on its own, and text
  you already typed is never replaced.

### Changed

- The chat system prompt says when to hand work to `/plan` or `/go`.
