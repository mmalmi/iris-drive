set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

export CARGO_TARGET_DIR := env_var_or_default("CARGO_TARGET_DIR", env_var("HOME") + "/.cache/cargo-target")

default:
    @just --list

info:
    @echo "Iris Drive commands"
    @echo
    @echo "Run"
    @echo "  just run"
    @echo "  just dev"
    @echo "  just run-linux"
    @echo "  just run-cli --help"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just linux-build"
    @echo "  just release"
    @echo "  just macos-xcodeproj"
    @echo "  just macos-build"
    @echo "  just dev-vms"
    @echo "  just smoke"
    @echo "  just smoke-macos"
    @echo "  just docker-cli-e2e"
    @echo
    @echo "Checks"
    @echo "  just test"
    @echo "  just structure"
    @echo "  just fmt"
    @echo "  just clippy"

run:
    @case "$(uname -s)" in \
        Darwin) ./scripts/macos-dev-app.sh run ;; \
        Linux) just run-linux ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

run-linux:
    ./tools/run-linux

dev:
    @case "$(uname -s)" in \
        Darwin) ./scripts/macos-dev-watch.sh ;; \
        *) echo "No local dev watcher for $(uname -s)." >&2; exit 1 ;; \
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

linux-build:
    cargo build -p idrive
    cd linux && cargo build

linux-install-menu:
    ./linux/scripts/install-dev-desktop.sh

macos-xcodeproj:
    cd macos && xcodegen generate

macos-build:
    xcodebuild -project macos/IrisDriveMac.xcodeproj -scheme IrisDriveMac -configuration Debug -derivedDataPath macos/.build/DerivedData CODE_SIGNING_ALLOWED=NO build

dev-vms *args:
    ./scripts/dev-vm-update-run.sh {{args}}

release:
    cargo build --workspace --release

test:
    cargo test --workspace

structure:
    ./scripts/check-rust-file-length.sh

docker-cli-e2e:
    ./scripts/docker-cli-e2e.sh

fmt:
    cargo fmt --all

clippy:
    just structure
    cargo clippy --workspace --all-targets -- -D warnings
