# Kohiro Roadmap

Each milestone is independently verifiable before moving on.

---

## Rust rewrite — Core server ✅

> `cargo build` produces a `kohiro` binary that serves git over SSH and manages per-repo myque tickets.

- [x] Rust crate with `russh` SSH transport and `clap` CLI.
- [x] SQLite auth store (`src/store.rs`): `users`, `ssh_keys`, `repos`, `repo_perms`.
- [x] Admin bootstrap from `--admin-key` / `--admin-user`.
- [x] Repo visibility flags: `--set-public owner/name`, `--set-private owner/name`.
- [x] Git serving via `git upload-pack` / `git receive-pack` child processes.
- [x] Access rules preserved: admin/owner/explicit grant can write; public repos are read-only; private repos deny outsiders.
- [x] Server-side tickets via `ssh host issues ...`, backed by myque at `data/myque/<owner>/<name>/.myque/`.
- [x] Go sources, git-bug integration, Wish/Bubble Tea middleware, and CI runner removed from the core build.

---

## Milestone 1 — SSH Foundation ✅

> `ssh -T -p 2222 user@localhost` connects and prints the Rust-port TUI-deferred hint.

- [x] `flake.nix` dev shell with Rust toolchain, git, sqlite, and native build tools.
- [x] `russh` server with host key generation at `data/.ssh/host_key`.
- [x] Accept-all public-key auth at the SSH layer; command handlers enforce authorization.

---

## Milestone 2 — Git Server ✅

> `git clone ssh://user@localhost:2222/owner/repo.git` and push/fetch work through the SSH server.

- [x] Bare repos stored at `data/repos/<owner>/<name>.git`.
- [x] Owner/admin push auto-creates bare repos and DB rows.
- [x] SSH channel data is streamed to `git upload-pack` / `git receive-pack`.

---

## Milestone 3 — Auth & Multi-user ✅

> Only authorized keys can push; anonymous/unknown keys can read public repos only.

- [x] SHA-256 OpenSSH fingerprint lookup in SQLite.
- [x] Admin and namespace owner get read-write access.
- [x] Explicit `repo_perms.write` grants get read-write access.
- [x] Public repos allow read-only access.
- [x] Hardened `owner/name(.git)` path parser rejects traversal and ambiguous paths.

---

## Milestone 4 — Tickets (myque) ✅

> `ssh host issues ...` manages server-side tickets for a repository.

- [x] `issues list owner/repo [--status S]`.
- [x] `issues show owner/repo <id>`.
- [x] `issues new owner/repo --title "..." [--label L] [--status S] [--agent A]`.
- [x] `issues move owner/repo <id> <status>`.
- [x] `issues board owner/repo`.

---

## Milestone 5 — Interactive TUI over SSH ✅

> `ssh -t -p 2222 user@localhost` opens a full-screen ratatui TUI rendered over the channel.

- [x] Repos tab: list accessible repos, create (`n`), delete (`d`/`x`), toggle public (`p`).
- [x] Keys tab: list / add (`a`) / remove (`d`/`x`) the signed-in user's SSH keys.
- [x] Repo detail view: Files browser + blob viewer, commit log, myque-backed Issues (`n` new, `e` edit body, `m` set status), and CI jobs/logs.
- [x] Rendered inline in the russh handler (`src/tui/`); non-PTY sessions print a hint.

> CI is no longer deferred: `.ci/push` jobs are MyQue tasks dispatched through `chilin`.

---

## Deferred / not yet ported

- [ ] TOML config file (`kohiro.toml`) for listen addr and data dir.
- [ ] SSH subcommand: `ssh host status` — server info, uptime, repo count.
- [ ] Admin UI or subcommands for managing users, keys, and repos.
- [ ] Graceful shutdown draining active git sessions.
- [ ] NixOS module / static binary packaging.
