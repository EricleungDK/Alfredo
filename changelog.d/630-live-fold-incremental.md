### Fixed

- The live `/go` fold stays exact for large files. A worker file past the 8 KiB output
  tail used to flood the task detail and agent view with raw code, or show a finished
  file as `… N earlier lines`. The fold is now counted as the text streams, so a long
  file shows `▸ calc.py · N lines so far`, finished files keep their rows, and Ctrl+O
  shows the retained tail under the open file's heading. The fold keeps at most 64
  rows; older files collapse into `… N earlier files`.
