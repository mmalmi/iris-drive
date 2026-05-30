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
    @echo "  just run-android"
    @echo "  just run-cli --help"
    @echo
    @echo "Build"
    @echo "  just build"
    @echo "  just linux-build"
    @echo "  just release"
    @echo "  just macos-xcodeproj"
    @echo "  just macos-build"
    @echo "  just android-build"
    @echo "  just android-install"
    @echo "  just android-smoke"
    @echo "  just ios-xcodeproj"
    @echo "  just ios-build"
    @echo "  just ios-smoke"
    @echo "  just lab"
    @echo "  just lab-smoke"
    @echo "  just lab-test"
    @echo "  just e2e"
    @echo "  just e2e-3vms"
    @echo "  just e2e-4devices"
    @echo "  just e2e-5devices"
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

run-android:
    ./tools/run-android install

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

android-build:
    ./tools/run-android build

android-install:
    ./tools/run-android install

android-smoke:
    ./scripts/mobile-android-smoke.sh

ios-xcodeproj:
    cd ios && xcodegen generate

ios-build:
    cd ios && xcodegen generate
    ./scripts/ios-simulator-smoke.sh --build-only

ios-smoke:
    ./scripts/ios-simulator-smoke.sh

dev-vms *args:
    ./scripts/dev-vm-update-run.sh {{args}}

lab *args:
    ./scripts/dev-lab.sh {{args}}

lab-smoke:
    ./scripts/dev-vm-smoke.sh

lab-test *args:
    just lab {{args}}
    just lab-smoke

e2e *args:
    ./scripts/e2e-everything-3vms.sh {{args}}

e2e-3vms *args:
    ./scripts/e2e-everything-3vms.sh {{args}}

e2e-4devices *args:
    ./scripts/cross-vm-four-platform-e2e.sh {{args}}

e2e-5devices *args:
    ./scripts/cross-vm-five-platform-e2e.sh {{args}}

release:
    cargo build --workspace --release

test:
    cargo test --workspace

structure:
    ./scripts/check-platform-parity-matrix.sh
    ./scripts/check-android-e2e-kit.sh
    ./scripts/check-ios-e2e-kit.sh
    ./scripts/check-source-file-size.sh

docker-cli-e2e:
    ./scripts/docker-cli-e2e.sh

fmt:
    cargo fmt --all

clippy:
    just structure
    cargo clippy --workspace --all-targets -- -D warnings
