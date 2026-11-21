### Security

- An always-allow rule for `write_file` or `edit_file` no longer covers CI and tooling
  config (`.github/`, `.husky/`, `.vscode/`, `.idea/`, `.devcontainer/`, `.cargo/`,
  `.gitlab-ci.yml`, `.pre-commit-config.yaml`, `.gitattributes`, `.gitmodules`): those
  edits always show an approval card with no `a`, and the card says why (#791).
  The check follows symlinks: a directory alias of `.github/` or a root-level protected
  name that links elsewhere is covered too.
