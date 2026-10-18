#!/usr/bin/env python3
"""Reuse canonical iOS assertions with receipt-verified prebuilt products."""
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2]).resolve()
output.mkdir(parents=True, exist_ok=True)


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('Canonical iOS harness changed; review prepared-product adapter')
    return text.replace(old, new)


for name in ['ios-gui-linking-smoke.sh', 'ios-simulator-smoke.sh']:
    text = (root / 'scripts' / name).read_text()
    text = replace_once(text, 'ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"',
                        'ROOT="${SOURCE_ROOT:?Required verified source checkout}"')
    text = replace_once(text, 'DERIVED_DATA="$ROOT/ios/.build/DerivedData"',
                        'DERIVED_DATA="${IRIS_DRIVE_PREPARED_IOS_DERIVED_DATA:?Required verified products}"')
    if name == 'ios-gui-linking-smoke.sh':
        begin = text.index('cargo build -p idrive\n')
        end = text.index('APP_PATH="$(resolve_app_path)"', begin)
        text = text[:begin] + '''[[ -x "$IDRIVE" ]] || { echo 'Missing verified native CLI' >&2; exit 2; }
DEVICE_UDID="$(select_simulator)"
DESTINATION="platform=iOS Simulator,id=$DEVICE_UDID"

''' + text[end:]
    else:
        text = replace_once(text, 'NO_BUILD=0', 'NO_BUILD=1')
        text = replace_once(text,
                            'if [[ "$INSTALL_ONLY" == 1 && ( "$BUILD_ONLY" == 1 || "$NO_BUILD" == 1 ) ]]; then',
                            'if [[ "$INSTALL_ONLY" == 1 && "$BUILD_ONLY" == 1 ]]; then')
        text = replace_once(text, '  cargo build -p idrive',
                            '  echo "Missing verified native CLI" >&2; exit 2')
    (output / name).write_text(text)
