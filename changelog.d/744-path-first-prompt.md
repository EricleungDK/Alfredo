### Fixed

- A chat prompt whose first word is a file path (`/etc/hosts what is this`, a dragged-in
  path, `//go`) is now sent to the model as chat text instead of answering `Unknown
  command`. A first word with a second `/` is never a command; mistyped commands such as
  `/gp` are still refused, and a running turn queues path-first prompts like other chat text.
