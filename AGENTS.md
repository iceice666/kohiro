# Agent Guideline

This file provides guidance to Agents when working with code in this repository.

## Project

Kohiro is a self-hosted git server that speaks SSH. Clients interact via standard `git` commands and (eventually) an interactive TUI over SSH. See `docs/ROADMAP.md` for milestone-by-milestone progress.

## Development environment

The project uses a Nix dev shell (`flake.nix`). Enter it with:

```sh
nix develop
```

This provides Go, gopls, goimports, git, git-bug, and sqlite. `GOPATH` is pinned to `.gopath/` inside the project root.

## Common commands

```sh
go run ./cmd/kohiro          # start the SSH server on :2222
go build ./...               # compile all packages
go test ./...                # run all tests
go vet ./...                 # static analysis
goimports -w .               # format + fix imports
```

Test the running server:

```sh
ssh -p 2222 user@localhost                              # greeting
git clone ssh://user@localhost:2222/owner/repo.git     # clone
```

## Architecture

### Middleware chain (`cmd/kohiro/main.go`)

`wish.NewServer` composes middleware in **reverse** order — the last listed runs first:

```
greetMiddleware  →  wishgit.Middleware  →  logging.Middleware
```

- **greetMiddleware** — prints a greeting, then calls `next`.
- **wishgit.Middleware** — handles `git-upload-pack` / `git-receive-pack` commands, serving repos from `./data/repos/`.
- **logging.Middleware** — logs every SSH session.

The `allowAllHooks` struct satisfies `wishgit.Hooks`; it grants `ReadWriteAccess` to every key and stubs `Push`/`Fetch`. Milestone 3 replaces this with a real auth lookup.

### Repo layout

Bare repos live at `data/repos/<owner>/<name>.git`. `git.Init(owner, name)` creates one via `wishgit.EnsureRepo`. Host keys are stored in `data/.ssh/`.

### Planned packages (not yet created)

Per the roadmap:
- `store/store.go` — SQLite (users, ssh_keys, repos, repo_perms, ci_runs)
- `auth/auth.go` — SSH fingerprint → user lookup, feeds into the wish public-key handler
- `ci/queue.go` — SQLite-backed job queue; receives notifications from `post-receive`
- `ci/runner.go` — `Runner` interface; `ShellRunner` execs `.ci/<event>` inside a container (podman/docker)
- TUI views wired via `wish/bubbletea` middleware (dispatches PTY sessions to Bubble Tea)
