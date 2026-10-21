### Fixed

- Chat `git_push` and the other git writes run without a controlling terminal, so an ssh
  host-key or passphrase prompt no longer draws over the TUI; the push fails fast and
  suggests ssh-agent instead.
