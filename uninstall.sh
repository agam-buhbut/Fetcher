#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons"

FILES=(
    "$BIN_DIR/taskmgr-cli"
    "$BIN_DIR/taskmgr-gui"
    "$APPS_DIR/taskmgr.desktop"
    "$ICON_DIR/taskmgr.png"
)

removed=0
for f in "${FILES[@]}"; do
    if [[ -e "$f" ]]; then
        rm "$f"
        echo "Removed $f"
        # NOT ((removed++)): that returns the pre-increment value, so the
        # first bump evaluates to 0 and `set -e` kills the script here.
        removed=$((removed + 1))
    fi
done

if [[ $removed -eq 0 ]]; then
    echo "Nothing to remove."
else
    echo "Done."
fi
