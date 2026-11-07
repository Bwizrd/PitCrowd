# Security

Please report a security problem privately, not in a public issue: use **Report a
vulnerability** on the repository's Security tab.

The parts worth a look:

- `jev_relay` holds the Jev API key so the browser never sees it. Anything that lets a key
  reach the page, a log or a response is a bug.
- `floor_tape` and `jev_relay` listen on 127.0.0.1 by default. The tape's ingest endpoint
  accepts prices from anything that can reach it, so do not expose it to a network you do not
  trust.

This is a local research and visualisation project that places no real orders, and it is
maintained in spare time, so a fix may take a few days.
