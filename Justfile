set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

default:
    @just --list

info:
    @echo "Iris Drive commands"
    @echo
    @echo "Run"
    @echo "  just run"
    @echo "  just run-cli --help"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just release"
    @echo "  just macos-xcodeproj"
    @echo "  just macos-build"
    @echo "  just smoke"
    @echo "  just smoke-macos"
    @echo
    @echo "Checks"
    @echo "  just test"
    @echo "  just fmt"
    @echo "  just clippy"

run:
    @case "$(uname -s)" in \
        Darwin) ./scripts/macos-dev-app.sh run ;; \
        Linux) just _run-daemon ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

smoke:
    @case "$(uname -s)" in \
        Darwin) just smoke-macos ;; \
        *) just test ;; \
    esac

smoke-macos:
    ./scripts/macos-smoke.sh

_run-daemon *args:
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

macos-xcodeproj:
    cd macos && xcodegen generate

macos-build:
    xcodebuild -project macos/IrisDriveMac.xcodeproj -scheme IrisDriveMac -configuration Debug CODE_SIGNING_ALLOWED=NO build

release:
    cargo build --workspace --release

test:
    cargo test --workspace

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings
