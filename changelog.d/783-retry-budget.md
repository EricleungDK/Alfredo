### Fixed

- Ctrl+R on a turn whose first attempt had already read a large file (about 64 KB or more of
  tool output) no longer fails at once with "Conversation output limit reached". The
  interrupted attempt's `⚙` rows are display only, so they no longer count against the live
  conversation cap; each is cut to what is drawn (200 lines, 2 KB) when retried, and saved
  earlier-attempt rows keep their own byte cap.
