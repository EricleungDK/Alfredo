### Fixed

- The task panel, agent view, chat context and worker commit message now single-quote check
  arguments that contain spaces or quotes (and show empty ones as `''`), as the `/plan` draft
  already did, so two checks split differently no longer read the same at `/approve`.
