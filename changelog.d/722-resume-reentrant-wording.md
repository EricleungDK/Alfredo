### Fixed

- `/resume` after a model-unreachable or context-too-small stop now says to start the
  model server or raise `--num-ctx`, then `/repair ID retry`, instead of only pointing at
  `/go GOAL`; `--help`, the README and the reference name both stops as the ones
  `--resume --headless` retries, and the reference says the retry counts as one repair
  beyond `--max-repairs` (#722).
