### Fixed

- Interactive `/go` in a repository whose Git config is refused now shows the whole
  refusal in the chat, with the `git config --local --unset KEY` command on its own row,
  and a short footer notice that leads with that command, so the 80- and 100-column
  footer no longer cuts the fix off.
