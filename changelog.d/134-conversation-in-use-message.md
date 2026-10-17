### Fixed

- Starting a second `alfredo-tui` while another owns the conversation no longer lands in
  the workspace selector with a truncated internal error. It exits with code 1 and says
  `Another alfredo-tui already owns conversation 'default' for this repo. Close it, or
  start this one with: alfredo-tui --conversation NAME`. Long errors in the selector now
  wrap in full.
