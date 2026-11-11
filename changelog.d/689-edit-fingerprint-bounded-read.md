### Security

- An approval card for `write_file` or `edit_file` no longer blocks on, or reads
  whole, the file it targets. A FIFO at the path is refused at once as `not a
  regular file` instead of hanging the chat turn, and a file over the 1 MiB edit
  limit is refused without being read: a 1 GiB target used to grow peak memory by
  about 1 GiB before the same refusal. The re-check before an approved write uses
  the same bounded read, so a file swapped for a FIFO after the card is refused
  too (#689).
