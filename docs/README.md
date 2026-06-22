# Kohiro

A tiny self-hosted Rust git server over SSH.

Current core features:

- git clone/fetch/push over SSH;
- SQLite-backed users, SSH keys, repos, and repo permissions;
- per-repo myque tickets via `ssh host issues ...`, stored at `data/myque/<owner>/<name>/.myque/`;
- interactive TUI over SSH (`ssh -t`): repos, SSH key management, file browser, commit log, and myque issues.

Deferred in the Rust port: container CI runner.
