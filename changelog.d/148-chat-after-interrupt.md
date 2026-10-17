### Fixed

- After Esc or a failed reply, a chat now accepts a new prompt and `/model NAME`
  instead of refusing both until Ctrl+R. The interrupted turn stays in the transcript
  marked `interrupted, not sent as context` and is not resent to the model; Ctrl+R
  still retries it until a new turn starts.
