Fetcher — Linux Task Manager
=============================

A Linux task manager with two frontends that share a common core library:

  taskmgr-cli   Terminal UI (ratatui / crossterm)
  taskmgr-gui   Desktop GUI (egui / eframe)

Both frontends expose the same four tabs: Processes, Performance,
Startup, and Services.


Requirements
------------
- Linux (uses /proc, nix signals, and sysinfo)
- systemd (required for the Services tab; the rest works without it)
- Wayland or X11 display server (GUI frontend only)
- Rust toolchain (https://rustup.rs)


Building
--------
Debug build (fast compile, slower binary):

    cargo build

Release build (optimised, stripped):

    cargo build --release

Binaries are written to:

    target/debug/taskmgr-cli
    target/debug/taskmgr-gui

    target/release/taskmgr-cli   (release)
    target/release/taskmgr-gui   (release)


Running
-------
    cargo run -p taskmgr-cli      # TUI
    cargo run -p taskmgr-gui      # GUI

Or run the compiled binaries directly after building.

Some actions (killing system processes, managing system-scope services)
require elevated privileges. Run with sudo if you hit permission errors.


Tabs
----
Processes
  Live list of all running processes. Updates every second.
  Columns: PID, User, Name, Status, CPU%, Memory, Disk R/s, Disk W/s.
  Per-process disk rates come from /proc/<pid>/io; the kernel hides
  other users' counters, so those rows show "—" unless run as root.

Performance
  Real-time gauges and scrolling history graphs for CPU usage,
  memory usage (plus swap, when present), disk I/O (read + write),
  and network I/O (RX + TX). History depth: 120 seconds.
  Disk I/O counts physical disks only — device-mapper/RAID layers
  and zram are excluded so stacked traffic isn't double-counted.

Startup
  XDG autostart entries for the current user and system-wide.
  Toggle entries on or off without editing files manually.

Services
  systemd service units. Switch between user and system scope.
  Start, stop, restart, enable, and disable units directly.


TUI Keybindings (taskmgr-cli)
------------------------------
Global
  1 / 2 / 3 / 4      Switch to Processes / Performance / Startup / Services
  Tab / Shift-Tab     Cycle tabs forward / backward
  ↑ ↓  or  j k        Move selection
  Page Down / Page Up  Move selection by 10
  g / Home            Jump to top
  G / End             Jump to bottom
  q  or  Esc          Quit
  Ctrl-C              Quit

Processes tab
  /                   Enter filter mode (type to filter by name, Enter to confirm, Esc to clear)
  s                   Sort by CPU%
  m                   Sort by memory
  p                   Sort by PID
  n                   Sort by name
  d                   Sort by disk read
  w                   Sort by disk write
  x  or  Delete       Send SIGTERM to selected process
  X                   Send SIGKILL to selected process

Startup tab
  Space               Toggle selected entry on / off
  r                   Refresh list

Services tab
  s                   Start selected service
  S                   Stop selected service
  R                   Restart selected service
  e                   Enable selected service
  E                   Disable selected service
  u                   Toggle between user and system scope
  r                   Refresh list


Project Layout
--------------
  core/       Shared OS-touching library (no UI dependencies)
  cli/        Terminal frontend
  gui/        Desktop GUI frontend
  assets/     Launcher icon installed by install.sh
  Cargo.toml  Workspace manifest
