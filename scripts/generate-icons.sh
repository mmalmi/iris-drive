#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SVG="$ROOT/linux/resources/iris-drive.svg"
MACOS_APPICON="$ROOT/macos/Resources/Assets.xcassets/AppIcon.appiconset"
WINDOWS_ICO="$ROOT/windows/IrisDrive.ico"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

render_png() {
  local size="$1"
  local output="$2"
  rsvg-convert -w "$size" -h "$size" "$SVG" -o "$output"
}

generate_macos_icons() {
  if [[ ! -d "$MACOS_APPICON" ]]; then
    echo "skipping macOS icons; missing $MACOS_APPICON"
    return
  fi

  render_png 16 "$MACOS_APPICON/icon_16x16.png"
  render_png 32 "$MACOS_APPICON/icon_16x16@2x.png"
  render_png 32 "$MACOS_APPICON/icon_32x32.png"
  render_png 64 "$MACOS_APPICON/icon_32x32@2x.png"
  render_png 128 "$MACOS_APPICON/icon_128x128.png"
  render_png 256 "$MACOS_APPICON/icon_128x128@2x.png"
  render_png 256 "$MACOS_APPICON/icon_256x256.png"
  render_png 512 "$MACOS_APPICON/icon_256x256@2x.png"
  render_png 512 "$MACOS_APPICON/icon_512x512.png"
  render_png 1024 "$MACOS_APPICON/icon_512x512@2x.png"
  echo "generated macOS app icons"
}

generate_windows_icon() {
  if [[ ! -d "$(dirname "$WINDOWS_ICO")" ]]; then
    echo "skipping Windows icon; missing $(dirname "$WINDOWS_ICO")"
    return
  fi

  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' RETURN

  local size
  for size in 16 24 32 48 64 128 256; do
    render_png "$size" "$tmpdir/icon_${size}.png"
  done

  node - "$tmpdir" "$WINDOWS_ICO" <<'NODE'
const fs = require("fs");
const path = require("path");

const [tmpDir, outPath] = process.argv.slice(2);
const sizes = [16, 24, 32, 48, 64, 128, 256];
const images = sizes.map((size) => ({
  size,
  data: fs.readFileSync(path.join(tmpDir, `icon_${size}.png`)),
}));
const headerSize = 6 + images.length * 16;
let offset = headerSize;
const header = Buffer.alloc(headerSize);

header.writeUInt16LE(0, 0);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(images.length, 4);

images.forEach((image, index) => {
  const entry = 6 + index * 16;
  header.writeUInt8(image.size === 256 ? 0 : image.size, entry);
  header.writeUInt8(image.size === 256 ? 0 : image.size, entry + 1);
  header.writeUInt8(0, entry + 2);
  header.writeUInt8(0, entry + 3);
  header.writeUInt16LE(1, entry + 4);
  header.writeUInt16LE(32, entry + 6);
  header.writeUInt32LE(image.data.length, entry + 8);
  header.writeUInt32LE(offset, entry + 12);
  offset += image.data.length;
});

fs.writeFileSync(outPath, Buffer.concat([header, ...images.map((image) => image.data)]));
NODE
  echo "generated Windows icon"
}

generate_android_icons() {
  local res_dir="$ROOT/android/app/src/main/res"
  if [[ ! -d "$res_dir" ]]; then
    echo "skipping Android icons; missing $res_dir"
    return
  fi

  local specs=(
    "mipmap-mdpi 48"
    "mipmap-hdpi 72"
    "mipmap-xhdpi 96"
    "mipmap-xxhdpi 144"
    "mipmap-xxxhdpi 192"
  )
  local spec density size dir
  for spec in "${specs[@]}"; do
    read -r density size <<<"$spec"
    dir="$res_dir/$density"
    mkdir -p "$dir"
    render_png "$size" "$dir/ic_launcher.png"
    render_png "$size" "$dir/ic_launcher_round.png"
  done

  echo "generated Android launcher icons"
}

generate_ios_icons() {
  local appicon_sets=()
  while IFS= read -r -d '' appicon_set; do
    appicon_sets+=("$appicon_set")
  done < <(find "$ROOT/ios" -path "*/Assets.xcassets/AppIcon.appiconset" -type d -print0 2>/dev/null)

  if [[ "${#appicon_sets[@]}" -eq 0 ]]; then
    echo "skipping iOS icons; no AppIcon.appiconset found under $ROOT/ios"
    return
  fi

  local appicon_set
  for appicon_set in "${appicon_sets[@]}"; do
    render_png 20 "$appicon_set/icon_20x20.png"
    render_png 40 "$appicon_set/icon_20x20@2x.png"
    render_png 60 "$appicon_set/icon_20x20@3x.png"
    render_png 29 "$appicon_set/icon_29x29.png"
    render_png 58 "$appicon_set/icon_29x29@2x.png"
    render_png 87 "$appicon_set/icon_29x29@3x.png"
    render_png 40 "$appicon_set/icon_40x40.png"
    render_png 80 "$appicon_set/icon_40x40@2x.png"
    render_png 120 "$appicon_set/icon_40x40@3x.png"
    render_png 120 "$appicon_set/icon_60x60@2x.png"
    render_png 180 "$appicon_set/icon_60x60@3x.png"
    render_png 76 "$appicon_set/icon_76x76.png"
    render_png 152 "$appicon_set/icon_76x76@2x.png"
    render_png 167 "$appicon_set/icon_83.5x83.5@2x.png"
    render_png 1024 "$appicon_set/icon_1024x1024.png"
    echo "generated iOS app icons in $appicon_set"
  done
}

need rsvg-convert
need node

generate_macos_icons
generate_windows_icon
generate_android_icons
generate_ios_icons
