### Fixed

- "build me a new app", "create me a new service", "make us a new app", "build a whole new
  app" and "create a completely new app" start Wayfinder again (and `--go` refuses them
  headless): `me`, `us`, `whole`, `completely`, `entirely` and `totally` count as filler
  between the creation verb and "new", and up to three filler words are allowed (was two).
