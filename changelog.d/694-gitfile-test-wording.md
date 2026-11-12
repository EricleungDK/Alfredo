### Fixed

- The sandboxed owner-gitfile test no longer pins one git version's wording: it accepts
  both "not a git repository" (git 2.43) and "not a regular file" (git 2.56) while still
  requiring a non-zero exit and no discovered repository.
