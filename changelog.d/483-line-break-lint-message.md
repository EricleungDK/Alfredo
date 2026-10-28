### Fixed

- A line break in a plan check argument is still refused, but the message now says so
  (`Plan task 1 check argument 4 contains a line break; put the code in a script file
  listed in the task's files, or write it on one line`) instead of describing a hidden or
  bidirectional character, so a planner model can fix it on the retry.
