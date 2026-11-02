### Security

- A HOST (escalated) `run_command` card no longer offers `a`, so every host run asks.
  Before, `a` made an exact host rule that later ran with no card, including push
  commands the `git push` check missed (`g''it push`, `git pu''sh`, `G=git; $G push`,
  `git -c alias.p=push p`) and a `./build.sh` run from another directory. Host rules
  already in `chat-allow.json` never match, and `/allow` lists them as inactive.
