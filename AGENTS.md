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

## Issue tracking (git-bug)

Issues live in git-bug refs inside this repo and mirror to github.com/iceice666/kohiro
via a configured bridge named `default`.

```sh
git-bug bug                           # list open bugs
git-bug bug show <id>                 # show one bug + comments
git-bug bug new --title="..." \
  --message="..." --non-interactive   # file a new issue
git-bug bug comment new <id> -m "..."
git-bug bug status close <id>

git-bug bridge pull default           # import new/updated GitHub issues
git-bug bridge push default           # push local changes upstream
git-bug push                          # publish refs/bugs/* + refs/identities/* to origin
git-bug pull                          # fetch bug refs from origin
```

`origin`'s fetch refspec includes `refs/bugs/*` and `refs/identities/*`, so a plain
`git fetch origin` keeps bug data in sync along with branches.

**First-time setup** (one-off; already done for the primary identity):
```sh
# 1. Create your identity (once per repo clone)
git-bug user new --non-interactive --name "Your Name" --email "you@example.com"

# 2. Configure the GitHub bridge (requires a PAT with `public_repo` scope)
git-bug bridge new --name=default --target=github \
  --owner=iceice666 --project=kohiro --token-stdin --non-interactive

# 3. Initial import and publish
git-bug bridge pull default
git-bug push origin
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
