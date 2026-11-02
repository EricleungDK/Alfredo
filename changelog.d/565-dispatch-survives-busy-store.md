### Fixed

- Automatic dispatch no longer switches itself off when the background task refresh finds the task store briefly busy: up to five consecutive busy refreshes are skipped (the last snapshot is kept); a sixth, or any other refresh error, still turns dispatch off with a visible message (#565).
