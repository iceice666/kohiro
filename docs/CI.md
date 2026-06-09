# Kohiro CI

Kohiro can run a CI script automatically every time you push to a repository. The script runs inside a container, isolated from the host, with your project files mounted read-only at `/work`.

## Quick start

Create two files in your repository and push:

```
.ci/push        ← script that runs on every push (required)
.ci/image       ← container image to use (optional, default: alpine:latest)
```

**Example `.ci/push`:**

```sh
#!/bin/sh
set -eux
cd /work
echo "build OK"
```

**Example `.ci/image`:**

```
golang:1.25-alpine
```

Once pushed, kohiro queues a CI run. The script executes inside the specified container with `/work` containing the exact tree at the pushed commit.

## Script conventions

- The runner executes `.ci/push` with `sh -c`, so the file does not need to be executable (no `chmod +x` required).
- `stdout` and `stderr` are merged into a single log file.
- Exit code `0` → **success**; any non-zero exit → **failed**.
- The working directory inside the container is `/work`. Your files are mounted read-only; write output elsewhere (e.g. a mounted volume or a temp dir inside the container).

## Skipping CI

If a push does not include `.ci/push` in the pushed commit tree, no CI run is created. Deleting `.ci/push` from a commit suppresses CI for that push.

## Container image

`.ci/image` is read from the pushed commit tree (not the current HEAD), so you can change the image in the same commit that changes your code. The value is trimmed of whitespace; if the file is absent or empty, `alpine:latest` is used.

Any image reachable by the configured container runtime (`podman`, `docker`, or `nerdctl`) is valid.

## Viewing results

### TUI

Open the interactive TUI (`ssh -p 2222 user@localhost`), navigate to a repository, and press `Tab` until the **CI** sub-tab is selected.

- The runs list shows the most recent 50 runs with status badge, short SHA, ref, and duration.
- Press `Enter` on a run to open the log viewer.
- While a run is active the log viewer refreshes every 500 ms.
- Press `Esc` to return to the list.

Status badges:

| Badge | Meaning |
|-------|---------|
| `✓`   | success |
| `✗`   | failed (non-zero exit) |
| `►`   | running |
| `!`   | error (infrastructure failure, e.g. image pull failed) |
| `·`   | queued |

### SSH log subcommand

Stream the log of the most recent run:

```sh
ssh -p 2222 user@host logs owner/repo
```

Stream the log of a specific run by ID:

```sh
ssh -p 2222 user@host logs owner/repo 42
```

If the run is still in progress, the command follows new output until the run finishes, then exits. If the run is already complete, the full log is printed and the command exits immediately.

## Run lifecycle

```
queued → running → success
                 → failed   (non-zero exit code)
                 → error    (infrastructure failure)
```

If the server restarts while a run is in the `running` state, it is marked `error` on the next startup. The queue then continues with the next `queued` run.

## Requirements

The server must have one of the following container runtimes available on `PATH` at startup (checked in priority order):

1. `podman`
2. `docker`
3. `nerdctl`

If none is found, the server refuses to start. The detected runtime is logged at startup.

## Limitations (M5)

- One run at a time — jobs are serialised in a single worker.
- Only the `push` event is supported. Tags and scheduled triggers are not yet implemented.
- The run captures HEAD at the time of push; if multiple refs are pushed in one operation only HEAD is used.
- Runs cannot be cancelled or retried from the TUI.
- Log files in `data/logs/` are not automatically rotated or cleaned up.
