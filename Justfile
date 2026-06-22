default: run

run:
    cargo run

build:
    cargo build

test:
    cargo test

fmt:
    cargo fmt

check:
    cargo fmt --check && cargo clippy -- -D warnings && cargo test
