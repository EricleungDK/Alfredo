### Security

- Escape Unicode C1 terminal controls in headless JSON stdout, including streamed tool
  events, without changing the values recovered by JSON consumers.
