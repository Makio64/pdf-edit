#!/usr/bin/env bash
# Build the Chrome extension and get Chrome ready to load it.
# Official Chrome builds only install an unpacked extension through their own "Load unpacked"
# button (the --load-extension switch is ignored since Chrome 137), so this script does every other
# step: build, open chrome://extensions and copy the extension folder path to the clipboard.
# Usage: scripts/install-extension.sh [--no-build]
set -euo pipefail

cd "$(dirname "$0")/.."
EXT_DIR="$(pwd)/extension"

if [[ "${1:-}" != "--no-build" ]]; then
  ./scripts/build-extension.sh
fi

printf '%s' "$EXT_DIR" | pbcopy
open -a "Google Chrome" "chrome://extensions/"

cat <<EOF

Extension folder (copied to the clipboard): $EXT_DIR

First time, in the chrome://extensions tab that just opened:
  1. Turn on "Developer mode" / « Mode développeur » (top right).
  2. Click "Load unpacked" / « Charger l'extension non empaquetée ».
  3. Press Cmd+Shift+G, paste (Cmd+V), press Return, then click "Select" / « Sélectionner ».

Already loaded? Click the reload arrow on the "PDF Edit" card to use the new build.
EOF
