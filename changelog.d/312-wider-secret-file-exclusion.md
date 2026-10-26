### Security

- Chat tools, `@path` mentions and planner context no longer read, list, search, edit or
  commit common credential files: `.npmrc`, `.netrc`, `.pgpass`, `.git-credentials`,
  `id_rsa*`, `id_ed25519*`, `id_ecdsa*`, `id_dsa*`, anything under `.ssh/`, `.gnupg/`,
  `.aws/` or `.kube/`, `*.env` such as `prod.env`, `credentials*.json`, `*.keystore`,
  `*.jks`, `*.tfvars`, and `secrets.yml` or `secrets.yaml`. Name matching ignores case.
  `git_commit` refuses them before asking, and the full list is in the Chat tools section
  of the reference.
- `git_diff` and `git_commit` also catch a secret renamed or copied to a plain name, even
  an edited copy in a repository with more than 1000 files; a listing git warns about is
  refused rather than trusted.
