### Fixed

- A model name without a tag (`qwen2.5-coder`) now matches the `:latest` model Ollama installs under it, in
  `--doctor`, `/model`, `/assign`, the `/models` picker highlight and the header `warm` state. A bare name still does
  not match another tag such as `:14b`. The name you typed is kept for display and requests.
