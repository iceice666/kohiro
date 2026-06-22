# Agent Guideline

This file provides guidance to Agents when working with code in this repository.

## Project

Kohiro is a self-hosted Rust git server that speaks SSH. Clients interact via standard `git` commands and the `issues` SSH subcommand. The interactive TUI and container CI runner are deferred in the Rust port. See `docs/ROADMAP.md` for milestone-by-milestone progress.

## Development environment

The project uses a Nix dev shell (`flake.nix`). Enter it with:

```sh
nix develop
```

This provides the Rust toolchain (`cargo`, `rustc`, `rustfmt`, `clippy`, `rust-analyzer`), `git`, `just`, `sqlite`, and native build tools for bundled SQLite / SSH crypto dependencies.

## Common commands

```sh
cargo run              # start the SSH server on :2222
cargo build            # compile the crate
cargo test             # run all tests
cargo fmt              # format Rust code
cargo clippy -- -D warnings
just check             # fmt check + clippy + tests
```

Test the running server:

```sh
ssh -T -p 2222 user@localhost                          # no-PTY hint
ssh -p 2222 user@localhost issues list owner/repo      # list tickets
git clone ssh://user@localhost:2222/owner/repo.git     # clone/fetch
```

## Issue tracking (myque)

Kohiro exposes per-repository tickets through SSH subcommands and stores them server-side with `myque`:

```sh
ssh host issues list owner/repo [--status ready]
ssh host issues show owner/repo <task-id>
ssh host issues new owner/repo --title "..." [--label bug] [--status backlog] [--agent coder]
ssh host issues move owner/repo <task-id> ready
ssh host issues board owner/repo
```

Ticket files live outside the bare git repo at `data/myque/<owner>/<name>/.myque/`. They do not travel with `git clone`. Read operations require repo read access; `new` and `move` require write access.

## Architecture

### SSH dispatch (`src/server.rs`)

Kohiro uses `russh` directly. Each connection gets a `Conn` handler:

- public-key auth accepts any presented key and records its SHA-256 fingerprint;
- `exec_request` dispatches `git-upload-pack` / `git upload-pack`, `git-receive-pack` / `git receive-pack`, and `issues ...`;
- git commands spawn the system `git` binary against bare repos under `data/repos/<owner>/<name>.git` and stream SSH channel data to/from the child process;
- `issues` commands call the myque-backed ticket dispatcher synchronously and return command output over SSH;
- PTY/shell sessions return a one-line TUI-deferred hint.

`auth::git_access`, `auth::can_read`, and `auth::can_write` enforce access: admin/owner/explicit-grant → read-write; public repo → read-only; else no access.

### Repo layout

Bare repos live at `data/repos/<owner>/<name>.git`. Host keys are stored in `data/.ssh/host_key`. SQLite lives at `data/kohiro.db`. Ticket stores live at `data/myque/<owner>/<name>/.myque/`.

### Packages

- `src/main.rs` — CLI flags, admin bootstrap, host key management, server startup.
- `src/server.rs` — `russh` server and SSH exec/channel handling.
- `src/store.rs` — SQLite users, SSH keys, repos, and repo permissions.
- `src/auth.rs` — key fingerprint lookup, repo path parsing, access decisions.
- `src/git.rs` — bare repo creation and git service command construction.
- `src/tickets.rs` — `issues` CLI parser and myque integration.
- `src/paths.rs` — centralized data path policy.
