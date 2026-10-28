### Security

- `/go`'s post-check guard now lists every changed tracked file, including both sides of a
  rename. A check that deleted an unapproved file and left an approved file that copied its
  content used to pass the guard as one rename, and the deletion was saved into the candidate.
  Such a run is now refused with "Check modified tracked files outside approved paths".
