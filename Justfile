default: run

# -- Dev --

run:
    go run ./cmd/kohiro

build:
    go build ./...

test:
    go test ./...

vet:
    go vet ./...

fmt:
    find . -name '*.go' -not -path './.gopath/*' 2>/dev/null | xargs goimports -w

check: fmt vet test

# -- git-bug --
# git-bug push/pull use go-git which bypasses the system SSH agent;
# use native git directly to push/pull bug refs instead.

bug-push:
    git push origin 'refs/identities/*' 'refs/bugs/*'

bug-pull:
    git fetch origin '+refs/bugs/*:refs/bugs/*' '+refs/identities/*:refs/identities/*'

bug-ls:
    git-bug bug

bug-sync:
    git-bug bridge pull default
    git-bug bridge push default
    just bug-push
