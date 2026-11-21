### Fixed

- `/undo` no longer leaves the interrupted turn's "Already applied" note listing an edit
  it restored: Ctrl+R and the next prompt stop telling the model that edit is still in
  place. The call keeps its `⚙` row, marked `undone`; an edit `/undo` refused stays in
  the note.
