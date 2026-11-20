### Fixed
- `/plan-revise` now refuses a request that would push the whole plan prompt (the draft's prompt plus ` | Revision request: ` and your request) past 8192 bytes, before asking the planner. The message names the combined size and the limit, and the draft is kept.
