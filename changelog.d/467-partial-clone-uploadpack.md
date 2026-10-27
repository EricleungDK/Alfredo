### Security

- Host git no longer lazy-fetches through a repository's partial clone. A config with
  `extensions.partialclone`, `remote.*.promisor`, a remote `uploadpack` or `receivepack`
  program, `core.sshcommand`, `core.gitproxy`, `protocol.allow` or `protocol.<name>.allow`
  is refused by the read-tier git tools and qualified before worker execution, naming the
  key. Host git runs with `GIT_NO_LAZY_FETCH=1` and `protocol.allow=never`, so a missing
  object is never fetched and no repository-chosen program runs on the host.
