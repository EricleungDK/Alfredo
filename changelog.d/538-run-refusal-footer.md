### Fixed

- A refused `/run` (or any other refused command) now puts its reason in the footer, such as
  `Task #1 is proposed; /approve 1 first` or `Dependency #1 is not accepted`, instead of
  only in the chat transcript that the task panel hides.
- Esc on a generating `/plan-revise` now says the request was stopped and the earlier draft
  was kept, and shows that draft in the plan-draft pane instead of `No tasks proposed yet`.
