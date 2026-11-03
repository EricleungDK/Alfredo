### Fixed

- The interactive `/go` and `/run` refusal for a bubblewrap that cannot create a sandbox now
  shows the whole sentence in chat rows (bwrap's error, "Allow bubblewrap to create user
  namespaces" and the README Troubleshooting pointer) and a footer that leads with the fix,
  instead of one line cut after about 100 characters. bwrap stderr ending in a period no
  longer produces `..` in the message (also in `--doctor`).
