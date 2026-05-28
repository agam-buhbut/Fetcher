#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"

echo "==> Building release binaries..."
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

mkdir -p "$BIN_DIR" "$APPS_DIR"

echo "==> Installing CLI..."
cp "$SCRIPT_DIR/target/release/taskmgr-cli" "$BIN_DIR/taskmgr-cli"
chmod +x "$BIN_DIR/taskmgr-cli"

echo "==> Installing GUI..."
cp "$SCRIPT_DIR/target/release/taskmgr-gui" "$BIN_DIR/taskmgr-gui"
chmod +x "$BIN_DIR/taskmgr-gui"

echo "==> Creating desktop entry..."
cat > "$APPS_DIR/taskmgr.desktop" << DESKTOP
[Desktop Entry]
Type=Application
Name=Task Manager
Comment=System monitor — processes, performance, startup, and services
Exec=$BIN_DIR/taskmgr-gui
Terminal=false
Categories=System;Monitor;
DESKTOP

echo ""
echo "Done."
echo "  CLI : $BIN_DIR/taskmgr-cli"
echo "  GUI : $BIN_DIR/taskmgr-gui"
echo "  App : $APPS_DIR/taskmgr.desktop"
echo ""

# Warn if ~/.local/bin is not in PATH
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "WARNING: $BIN_DIR is not in your PATH."
    echo "Add this to ~/.bashrc (or ~/.zshrc) and restart your shell:"
    echo ""
    echo '  export PATH="$HOME/.local/bin:$PATH"'
    echo ""
fi
