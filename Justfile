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
    @echo
    @echo "Checks"
    @echo "  just test"
    @echo "  just fmt"
    @echo "  just clippy"

run:
    @case "$(uname -s)" in \
        Darwin) just _run-macos ;; \
        Linux) just _run-daemon ;; \
        *) echo "No local run target for $(uname -s). Use just --list for available commands." >&2; exit 1 ;; \
    esac

_run-macos:
    @just macos-xcodeproj; \
    target_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"; \
    cargo build -p idrive; \
    xcodebuild -project macos/IrisDriveMac.xcodeproj -scheme IrisDriveMac -configuration Debug CODE_SIGNING_ALLOWED=NO build >/tmp/iris-drive-macos-build.log; \
    app_path="$(xcodebuild -project macos/IrisDriveMac.xcodeproj -scheme IrisDriveMac -configuration Debug -showBuildSettings 2>/dev/null | awk -F' = ' '/^[[:space:]]*BUILT_PRODUCTS_DIR = / { dir=$2 } /^[[:space:]]*FULL_PRODUCT_NAME = / { app=$2 } END { print dir "/" app }')"; \
    if [[ -z "$app_path" || ! -d "$app_path" ]]; then \
        echo "Built macOS app not found. Build log: /tmp/iris-drive-macos-build.log" >&2; \
        exit 1; \
    fi; \
    cp "$target_dir/debug/idrive" "$app_path/Contents/MacOS/idrive"; \
    chmod +x "$app_path/Contents/MacOS/idrive"; \
    codesign --force --sign - --entitlements macos/IrisDriveMac.entitlements "$app_path/Contents/MacOS/idrive"; \
    codesign --force --sign - --entitlements macos/FileProvider/FileProvider.entitlements "$app_path/Contents/PlugIns/IrisDriveFileProvider.appex"; \
    codesign --force --sign - --entitlements macos/IrisDriveMac.entitlements "$app_path"; \
    codesign --verify --strict --deep "$app_path"; \
    open "$app_path"; \
    echo "macOS app launched: $app_path"

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
