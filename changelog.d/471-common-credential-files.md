### Security

- Common credential files are excluded from every chat tool, `@path` mentions and planner
  context, and refused by git_commit: `.pypirc`, `.htpasswd`, `.vault-token`, `.s3cfg`,
  `.boto`, `.dockercfg`, `kubeconfig` (and `.yaml`/`.yml`),
  `application_default_credentials.json`, Terraform state (`*.tfstate`, `*.tfstate.backup`
  and timestamped backups), `.docker/config.json`, `.cargo/credentials`,
  `.gem/credentials`, `.config/gh`, `.azure`, `.password-store`, `service-account*.json`,
  `client_secret*.json`, `*.p8`, `*.ppk` and `*.gpg`.
