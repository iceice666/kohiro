# Kohiro Roadmap

Each milestone is independently verifiable before moving on.

---

## Milestone 1 — SSH Foundation ✅
> `ssh -p 2222 user@localhost` connects and prints a greeting.

- [x] `flake.nix` dev shell (go, git, git-bug, sqlite)
- [x] `wish` server, host key generation, accept-all public key auth
- [x] Greeting middleware + logging

---

## Milestone 2 — Git Server ✅
> `git clone ssh://user@localhost:2222/owner/repo.git` works.

- [x] `wish/git` middleware wired to `data/repos/`
- [x] `git/repo.go`: `Init(owner, name string)` — creates bare repo at `data/repos/<owner>/<name>.git`
- [x] Stub access hook (allow all for now)
- [x] `post-receive` hook stub (trigger point for CI, no-op for now)

---

## Milestone 3 — Auth & Multi-user ✅
> Only keys in the DB can push; unknown keys get read-only on public repos.

- [x] SQLite store (`store/store.go`): open, migrate
- [x] Schema: `users`, `ssh_keys`, `repos`, `repo_perms`
- [x] Bootstrap: first admin key from `--admin-key` flag (with optional `--admin-user`, default "admin")
- [x] `auth/auth.go`: fingerprint → user lookup, wire into wish public key handler
- [x] Access hook: owner/write → push allowed; public repo → fetch allowed; else deny
- [x] `--set-public owner/name` / `--set-private owner/name` flags to toggle repo visibility

---

## Milestone 4 — TUI
> Bare `ssh -p 2222 user@localhost` opens an interactive terminal UI.

- [x] `wish/bubbletea` middleware; dispatch: PTY → TUI, command → passthrough
- [x] View: repo list (own + public)
- [x] View: file browser (tree + blob via `go-git` read-only)
- [x] View: commit log
- [x] View: SSH key management (list, add, remove own keys)
- [x] View: repo management (create, delete, toggle public/private)

---

## Milestone 5 — CI
> Push to a repo with `.ci/push` → job runs in a container → status visible in TUI.

- [ ] Schema: `ci_runs` (id, repo_id, sha, ref, status, queued/started/finished timestamps)
- [ ] `ci/queue.go`: SQLite-backed queue, channel notify, recover stale runs on restart
- [ ] `ci/runner.go`: `Runner` interface; `ShellRunner` shells out to `podman`/`docker`/`nerdctl`
  - Reads `.ci/image` (default: `alpine:latest`)
  - Mounts repo at `/work`, execs `.ci/<event>` inside container
  - Streams stdout/stderr → `data/logs/<run-id>.log`
- [ ] Wire `post-receive` hook → `queue.Enqueue`
- [ ] TUI view: CI run list per repo, log viewer (tail)
- [ ] SSH subcommand: `ssh host logs <owner>/<repo> [run-id]`

---

## Milestone 6 — Issues (git-bug)
> Issues are stored inside the repo's git objects; visible and manageable from the TUI.

- [ ] Shell out to `git-bug` binary for read (list, show)
- [ ] TUI view: issue list, issue detail
- [ ] TUI action: create issue, add comment, close

---

## Milestone 7 — Polish
> Ready for daily use.

- [ ] TOML config file (`kohiro.toml`) for addr, data dir, container runtime
- [ ] SSH subcommand: `ssh host status` — server info, uptime, repo count
- [ ] Admin TUI pane: manage users, revoke keys, delete repos
- [ ] Graceful shutdown: drain CI queue, close SSH sessions
- [ ] Single static binary, documented `flake.nix` NixOS module (optional)
