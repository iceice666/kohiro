# Kohiro

A tiny self-hosted Rust git server over SSH.

Current core features:

- git clone/fetch/push over SSH;
- SQLite-backed users, SSH keys, repos, and repo permissions;
- per-repo myque tickets via `ssh host issues ...`, stored at `data/myque/<owner>/<name>/.myque/`;
- interactive TUI over SSH (`ssh -t`): repos, SSH key management, file browser, commit log, myque issues, and CI jobs;
- MyQue-backed push CI: `.ci/push` is stored as a ticket template and dispatched through `chilin`.

Deferred in the Rust port: admin UI, server status command, graceful shutdown, and packaging.
