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
ssh -p 2222 user@localhost                              # opens TUI
ssh -T -p 2222 user@localhost                          # no-PTY hint
git clone ssh://user@localhost:2222/owner/repo.git     # clone
```

## Architecture

### Middleware chain (`cmd/kohiro/main.go`)

`wish.NewServer` composes middleware in **reverse** order — the last listed runs first:

```
tui.Middleware  →  wishgit.Middleware  →  logging.Middleware
```

- **tui.Middleware** — if the session has a PTY, launches the Bubble Tea TUI. Otherwise calls `next` so git ops fall through. Non-PTY sessions with no command receive a one-liner hint.
- **wishgit.Middleware** — handles `git-upload-pack` / `git-receive-pack` commands, serving repos from `./data/repos/`.
- **logging.Middleware** — logs every SSH session.

`hooks := auth.New(st)` is the `wishgit.Hooks` implementation. It enforces per-repo access: admin/owner/explicit-grant → read-write; public repo → read-only; else no access.

### Repo layout

Bare repos live at `data/repos/<owner>/<name>.git`. `git.Init(owner, name)` creates one via `wishgit.EnsureRepo`. Host keys are stored in `data/.ssh/`.

### Packages

- `store/store.go` — SQLite (users, ssh_keys, repos, repo_perms)
- `auth/auth.go` — SSH fingerprint → user lookup; `Hooks.AuthRepo` enforces access; `Hooks.UserFromSession` resolves the TUI user; `Hooks.CanRead` is shared by TUI and git path
- `git/repo.go` — `RepoDir`, `Init`
- `git/read.go` — `OpenRepo`, `CommitLog`, `HeadTree`, `TreeAt`, `Blob`, `IsBinary` via go-git
- `tui/` — Bubble Tea TUI (Repos tab, Keys tab, file browser, commit log); entry via `tui.Middleware`
- `ci/queue.go` — (planned M5) SQLite-backed job queue
- `ci/runner.go` — (planned M5) `Runner` interface; `ShellRunner` execs `.ci/<event>` inside a container
