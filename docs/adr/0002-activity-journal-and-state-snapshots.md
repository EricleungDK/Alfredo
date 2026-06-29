# Keep an Activity Journal alongside state snapshots

The Mission Control App will persist canonical current-state snapshots and a separate append-only Activity Journal for meaningful actions. This preserves attribution, auditability, and Agent Console narration without adopting full event sourcing or retaining high-volume transient output such as token streams and terminal bytes as domain events.
