#!/usr/bin/env bash
set -euo pipefail

# Node vendor SHASUMS256, .NET release metadata, Gradle release checksums,
# npm registry integrity and the crates.io index pin these exact archives.
tool="$1"
case "$tool:$(uname -s):$(uname -m)" in
  node:Linux:x86_64)
    archive=node-v24.22.2-linux-x64.tar.xz
    hash=48cc0e21e048790e54c0cf5749c2725005313d652651c40b7a21feb9697aaf75
    ;;
  node:Darwin:arm64)
    archive=node-v24.22.2-darwin-arm64.tar.gz
    hash=aa9549349e47dffaa42dcf95d6e06ade8c4bdafc980979883b88767670df385a
    ;;
  node:MINGW*:x86_64)
    archive=node-v24.22.2-win-x64.zip
    hash=ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2
    ;;
  dotnet:MINGW*:x86_64)
    archive=dotnet-sdk-8.0.428-win-x64.zip
    hash=cce1dc5cfb2c5b73ddd9d3658f6009deb85be3a58dbec44a6dfe3910bb32b044ad2fc6bb28c77aded70c73270a204d207fe790dcb82ffca6c7b42c1335b51c21
    ;;
  gradle:Linux:x86_64|gradle:Darwin:arm64)
    archive=gradle-8.13-bin.zip
    hash=20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78
    ;;
  pnpm:Linux:x86_64)
    archive=pnpm-10.23.0.tgz
    hash=21c4e5698002ade97e4efe8b8b4a89a8de3c85a37919f957e7a0f30f38fbc5bbdd05980ffe29179b2fb6e6e691242e098d945d1601772cad0fef5fb6411e2a4b
    ;;
  cargo-deb:Linux:x86_64)
    archive=cargo-deb-3.8.0.crate
    hash=201780f7056812fe4d8078b2772c1a222c56fa924cd2aafa1aaf74ff2777e188
    ;;
  *) printf 'Unsupported hosted tool or platform: %s\n' "$tool" >&2; exit 2 ;;
esac

case "$tool" in
  node) url="https://nodejs.org/dist/v24.22.2/$archive" ;;
  dotnet) url="https://builds.dotnet.microsoft.com/dotnet/Sdk/8.0.428/$archive" ;;
  gradle) url="https://services.gradle.org/distributions/$archive" ;;
  pnpm) url="https://registry.npmjs.org/pnpm/-/$archive" ;;
  cargo-deb) url="https://static.crates.io/crates/cargo-deb/$archive" ;;
esac

base="$RUNNER_TEMP"
if [[ "$RUNNER_OS" == Windows ]]; then
  base="$(cygpath -u "$base")"
fi
work="$(mktemp -d "$base/iris-drive-$tool.XXXXXXXX")"
curl --proto '=https' --proto-redir '=https' --fail --location --silent --show-error \
  --retry 2 --max-time 360 --max-filesize 350000000 --output "$work/$archive" "$url"
algorithm=256
if [[ "${#hash}" == 128 ]]; then algorithm=512; fi
(
  cd "$work"
  if [[ "$RUNNER_OS" == macOS ]]; then
    printf '%s  %s\n' "$hash" "$archive" | shasum -a "$algorithm" -c
  else
    printf '%s  %s\n' "$hash" "$archive" | "sha${algorithm}sum" -c
  fi
)
case "$archive" in
  *.zip) unzip -q "$work/$archive" -d "$work" ;;
  *) tar -xf "$work/$archive" -C "$work" ;;
esac

case "$tool:$RUNNER_OS" in
  node:Windows) bin="$work/${archive%.zip}" ;;
  node:Linux) bin="$work/${archive%.tar.xz}/bin" ;;
  node:macOS) bin="$work/${archive%.tar.gz}/bin" ;;
  dotnet:Windows)
    bin="$work"
    printf 'DOTNET_ROOT=%s\nDOTNET_MULTILEVEL_LOOKUP=0\n' "$(cygpath -w "$bin")" >> "$GITHUB_ENV"
    ;;
  gradle:*) bin="$work/gradle-8.13/bin" ;;
  pnpm:*)
    bin="$work/package/bin"
    ln -s pnpm.cjs "$bin/pnpm"
    ;;
  cargo-deb:*)
    cargo install --locked --jobs 2 --path "$work/cargo-deb-3.8.0" --root "$work"
    bin="$work/bin"
    ;;
esac
if [[ "$RUNNER_OS" == Windows ]]; then
  bin="$(cygpath -w "$bin")"
fi
printf '%s\n' "$bin" >> "$GITHUB_PATH"
