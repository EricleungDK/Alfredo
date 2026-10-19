### Added

- The saved plan receipt now keeps the planner's prompt and completion token counts and
  model time (`usage`) when the server reports them, and the `/go` finish notice and
  summary add a bounded line such as `tokens 12.3k in / 1.1k out · model 38 s`, summed
  over the planner, workers and repairs of that run. The line is left out when the
  server reports no counts. `eval/run_eval.py` warns (`planner_token_mismatch`) when the
  planner tokens saved in state differ from the server's count.
