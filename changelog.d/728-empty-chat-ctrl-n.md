### Changed

- Ctrl+N on an empty chat (no messages, empty draft, no running turn) no longer adds another
  empty chat; it says `This chat is empty; type here, or Tab to another chat`. The
  eight-chat limit notice now says `/clear` empties a chat or `--conversation NAME` starts
  another set, and the `/clear` notice no longer promises Ctrl+N when eight chats exist.
  The prompt title shows `Tab chats` when two or more chats exist.
