# Kohiro CI

CI is deferred in the Rust port.

The previous Go implementation queued `.ci/push` jobs and ran them in a container. That code and schema are intentionally not part of the current core Rust server. The active Rust binary serves git over SSH and exposes myque-backed `issues` subcommands only.

Planned CI port, when resumed:

- detect `.ci/push` on push;
- queue runs in SQLite or a replacement queue store;
- execute inside a configured container runtime;
- store logs under `data/`;
- expose status through the future TUI and/or SSH subcommands.
