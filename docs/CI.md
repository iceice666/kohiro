# Kohiro CI

Kohiro CI is MyQue-backed. A pushed repository may contain `.ci/push`; Kohiro treats that file as a MyQue ticket template, copies it into the repository's server-side ticket store, fills push context placeholders, and dispatches it through MyQue using the `chilin` backend.

Push flow:

- after a successful `git receive-pack`, Kohiro checks for `.ci/push` at `HEAD`;
- if present, Kohiro checks out `HEAD` under `data/ci/work/<owner>/<repo>/<task-id>`;
- Kohiro creates a ready MyQue task under `data/myque/<owner>/<repo>/.myque/tasks/`;
- MyQue parses frontmatter and dispatches only CI tasks labelled `ci:push`;
- `chilin::ChilinRunner` parses the Markdown body: each executable H2 section is a Chilin step, leading `>` metadata before the first H2 is task metadata, and leading `>` metadata under a step is step metadata;
- logs are written under `data/ci/logs/<owner>/<repo>/` and shown through `ssh host ci ...` plus the TUI CI tab.

`.ci/push` should be a MyQue-compatible Markdown ticket. Its frontmatter routes through `backend = "chilin"`; its body carries Chilin metadata and executable H2 steps, for example:

````md
+++
title = "CI push"
labels = ["safe-auto", "ci", "ci:push"]
agent = "ci"
backend = "chilin"
+++

> log_path = "{log_path}"
> mount.source = "{workdir}"
> mount.target = "/repo"
> mount.readonly = false
> env.CI_REPO = "{repo}"
> env.CI_SHA = "{sha}"
> env.CI_PUSHER = "{pusher}"

## Goal

Run push CI for {repo}.

## Context

Commit {sha} was pushed by {pusher}.

## Constraints

Run in the checked-out pushed commit.

## Acceptance

The command exits successfully.

## Test

```sh
./ci/run.sh
```
````

Supported placeholders: `{repo}`, `{owner}`, `{name}`, `{sha}`, `{pusher}`, `{workdir}`, `{log_path}`.
