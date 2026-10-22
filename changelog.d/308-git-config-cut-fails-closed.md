### Security

- A repository whose git config is too large to check is now refused by the chat git
  tools (`git_status`, working-tree and staged `git_diff`, and the git write tools) instead
  of being trusted in part. Padding a `.git/config` past 1 MiB could hide a filter driver,
  which then ran on the host outside the sandbox.
- Large diffs whose file list passes 4 MiB are refused by `git_diff` rather than checked
  only in part, so a renamed secret cannot slip past the secret exclusions.
