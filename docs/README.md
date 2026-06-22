# Kohiro

A tiny self-hosted Rust git server over SSH.

Current core features:

- git clone/fetch/push over SSH;
- SQLite-backed users, SSH keys, repos, and repo permissions;
- per-repo myque tickets via `ssh host issues ...`, stored at `data/myque/<owner>/<name>/.myque/`.

Deferred in the Rust port: interactive TUI and container CI runner.
