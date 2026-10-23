### Fixed

- An autopilot pause caused by a command that could not be recorded (for example
  `Command history capacity reached`) now keeps its reason as autopilot's own notice:
  `/autopilot`, the saved state and headless stderr name it instead of the generic pause
  text, and the headless JSON lists it under `failed` as `{id:null,title:"autopilot",reason}`
  (additive; a plain pause with only the generic text still reports `failed:[]`).
