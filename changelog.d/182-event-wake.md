### Changed
- The terminal loop now wakes as soon as a model reply, streamed chunk, planner, worker or task result is ready instead of after its 33 ms input poll: reply to screen drops from about 31 ms to about 2 ms and Enter to request from about 38 ms to about 5 ms, with unchanged idle CPU.
