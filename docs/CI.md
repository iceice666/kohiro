# Kohiro CI

Kohiro CI is MyQue-backed. A pushed repository may contain `.ci/push`; Kohiro treats that file as a MyQue ticket template, copies it into the repository's server-side ticket store, fills push context placeholders, and dispatches it through MyQue using the `chilin` backend.

Push flow:

- after a successful `git receive-pack`, Kohiro checks for `.ci/push` at `HEAD`;
- if present, Kohiro checks out `HEAD` under `data/ci/work/<owner>/<repo>/<task-id>`;
- Kohiro creates a ready MyQue task under `data/myque/<owner>/<repo>/.myque/tasks/`;
- MyQue dispatch selects only CI tasks labelled `ci:push`;
- `chilin::ChilinRunner` parses the task's `## Chilin` section and runs the configured command through the configured shell/container runner;
- logs are written under `data/ci/logs/<owner>/<repo>/` and shown through `ssh host ci ...` plus the TUI CI tab.

`.ci/push` should be a MyQue-compatible Markdown ticket. Its body must include the standard MyQue sections required for auto-dispatch and a `## Chilin` TOML block, for example:

````md
+++
title = "CI push"
labels = ["safe-auto", "ci", "ci:push"]
agent = "ci"
backend = "chilin"
+++

## Goal

Run push CI for {repo}.

## Context

Commit {sha} was pushed by {pusher}.

## Constraints

Run in the checked-out pushed commit.

## Acceptance

The command exits successfully.

## Chilin

```toml
command = ["sh", "-c", "./ci/run.sh"]
env = [["CI_REPO", "{repo}"], ["CI_SHA", "{sha}"], ["CI_PUSHER", "{pusher}"]]
log_path = "{log_path}"

[mount]
source = "{workdir}"
target = "/repo"
readonly = false
```
````

Supported placeholders: `{repo}`, `{owner}`, `{name}`, `{sha}`, `{pusher}`, `{workdir}`, `{log_path}`.
