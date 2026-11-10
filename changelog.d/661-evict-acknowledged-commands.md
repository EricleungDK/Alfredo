### Fixed

- A long conversation no longer wedges at "Command history capacity reached" after
  about 360 to 390 task commands: acknowledged `Submitted` commands (receipt present,
  nothing running) are now evictable like other settled commands. The saved file format
  is unchanged.
