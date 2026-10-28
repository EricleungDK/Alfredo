### Fixed

- Starting Alfredo is faster. The bounded `git` calls that run before the
  first frame no longer sleep a fixed 5 ms between checks; they wake as soon as git's
  output ends and then check for its exit at 100 microsecond steps that grow to 5 ms.
  The 15 s limit, the stop on a cancelled round and the output limit are unchanged.
