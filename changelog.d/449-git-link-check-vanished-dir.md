### Fixed

- A chat git tool call that finds its repository's object directories changing or
  unreadable while it checks them for links (a background `git gc` or repack) is still
  refused, but now says so instead of blaming the call's time limit. The large-index
  secret-copy test no longer lets a fixture commit start a background `git gc`
  (#449).
