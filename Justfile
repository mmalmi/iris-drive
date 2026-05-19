set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

info:
    @echo "Iris Drive commands"
    @echo
    @echo "Run"
    @echo "  just run"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just release"
    @echo
    @echo "Checks"
    @echo "  just test"
    @echo "  just fmt"
    @echo "  just clippy"

run *args:
    cargo run -p idrive -- {{args}}

build:
    cargo build --workspace

release:
    cargo build --workspace --release

test:
    cargo test --workspace

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings
