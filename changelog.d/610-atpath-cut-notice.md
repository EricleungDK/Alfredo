### Fixed

- A file cut to the 32 KiB per-prompt `@path` budget now gets one dim row
  (`@big.txt: attached 32768 of 66890 bytes (32 KiB per-prompt limit; the model sees the
  first part)`), also on stderr with `--chat --headless`. A file that fits stays silent,
  and the footer still counts only mentions that were not attached.
