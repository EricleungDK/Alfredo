### Changed

- A chat model that repeats the same failing read call now gets a note on the third and later identical `error:` results of a turn, telling it to change the arguments or answer from what it knows. The turn and the round cap are unchanged.
