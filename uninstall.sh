#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"

FILES=(
    "$BIN_DIR/taskmgr-cli"
    "$BIN_DIR/taskmgr-gui"
    "$APPS_DIR/taskmgr.desktop"
)

removed=0
for f in "${FILES[@]}"; do
    if [[ -e "$f" ]]; then
        rm "$f"
        echo "Removed $f"
        ((removed++))
    fi
done

if [[ $removed -eq 0 ]]; then
    echo "Nothing to remove."
else
    echo "Done."
fi
