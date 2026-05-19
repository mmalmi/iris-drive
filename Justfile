set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

info:
    @echo "Iris Drive commands"
    @echo
    @echo "Run"
    @echo "  just run"
    @echo "  just run-macos"
    @echo "  just run-daemon"
    @echo "  just run-cli --help"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just release"
    @echo
    @echo "Checks"
    @echo "  just test"
    @echo "  just fmt"
    @echo "  just clippy"

run:
    @case "$(uname -s)" in \
        Darwin) just run-macos ;; \
        Linux) just run-daemon ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

run-macos:
    @profile="${IRIS_DRIVE_PROFILE:-debug}"; \
    build_profile_arg=""; \
    profile_dir="debug"; \
    if [[ "$profile" == "release" ]]; then \
        build_profile_arg="--release"; \
        profile_dir="release"; \
    elif [[ "$profile" != "debug" ]]; then \
        echo "IRIS_DRIVE_PROFILE must be 'debug' or 'release'." >&2; \
        exit 2; \
    fi; \
    target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"; \
    cargo build $build_profile_arg -p idrive -p iris-drive-mac; \
    exec "$target_dir/$profile_dir/iris-drive-mac"

run-daemon *args:
    @profile="${IRIS_DRIVE_PROFILE:-debug}"; \
    build_profile_arg=""; \
    profile_dir="debug"; \
    if [[ "$profile" == "release" ]]; then \
        build_profile_arg="--release"; \
        profile_dir="release"; \
    elif [[ "$profile" != "debug" ]]; then \
        echo "IRIS_DRIVE_PROFILE must be 'debug' or 'release'." >&2; \
        exit 2; \
    fi; \
    target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"; \
    cargo build $build_profile_arg -p idrive; \
    exec "$target_dir/$profile_dir/idrive" daemon {{args}}

run-cli *args:
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
