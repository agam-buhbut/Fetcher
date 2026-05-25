//! End-to-end verification of mutating core APIs against dummy artifacts.
//!
//! Run with: `cargo run --example verify -p taskmgr-core`
//!
//! Exercises three side-effecting paths, in isolation from real system state:
//!   1. `kill_process` — spawn `sleep 9999`, kill via SIGTERM, confirm exit.
//!   2. `startup::set_enabled` — write a dummy `.desktop`, list it, toggle
//!      Hidden=true/false, verify on-disk content.
//!   3. `services::service_action` — install a dummy `--user` unit, drive
//!      Start / Stop / Enable / Disable via D-Bus, verify with `systemctl --user`.
//!
//! All dummy files are removed at the end, regardless of success.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use taskmgr_core::services::{service_action, ServiceOp, ServiceScope};
use taskmgr_core::startup::{list_entries, set_enabled, AutostartScope};
use taskmgr_core::{kill_process, KillSignal};

const TAG: &str = "fetcher-verify";

fn main() {
    println!();
    println!("=== fetcher verify ===");
    println!();

    let mut failures: u32 = 0;

    println!("[1/3] kill_process");
    report(
        "spawn sleep + SIGTERM + confirm exit",
        verify_kill(),
        &mut failures,
    );

    println!();
    println!("[2/3] autostart (.desktop)");
    report(
        "create + list + disable + enable",
        verify_autostart(),
        &mut failures,
    );

    println!();
    println!("[3/3] services (systemd --user)");
    report(
        "install + start + stop + enable + disable",
        verify_services(),
        &mut failures,
    );

    println!();
    if failures == 0 {
        println!("=== ALL OK ===");
        std::process::exit(0);
    } else {
        println!("=== {failures} FAILURE(S) ===");
        std::process::exit(1);
    }
}

fn report(label: &str, result: Result<(), String>, failures: &mut u32) {
    match result {
        Ok(()) => println!("  {label:<50} OK"),
        Err(e) => {
            println!("  {label:<50} FAIL");
            for line in e.lines() {
                println!("    -> {line}");
            }
            *failures += 1;
        }
    }
}

// ---------------------------------------------------------------- kill ----

fn verify_kill() -> Result<(), String> {
    let mut child = Command::new("sleep")
        .arg("9999")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn sleep: {e}"))?;
    let pid = child.id();

    // Confirm it's alive before we kill it, so we know we exercised the path.
    thread::sleep(Duration::from_millis(50));
    if child
        .try_wait()
        .map_err(|e| format!("try_wait: {e}"))?
        .is_some()
    {
        return Err("sleep exited before kill (env issue)".into());
    }

    kill_process(pid, KillSignal::Term).map_err(|e| format!("kill_process: {e}"))?;

    // Wait up to 1s for exit.
    for _ in 0..20 {
        if child
            .try_wait()
            .map_err(|e| format!("try_wait: {e}"))?
            .is_some()
        {
            println!("    pid {pid} exited cleanly after SIGTERM");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    // Last-ditch cleanup so we don't leak the sleep.
    let _ = child.kill();
    Err(format!("pid {pid} still alive 1s after SIGTERM"))
}

// ----------------------------------------------------------- autostart ----

fn autostart_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| "neither XDG_CONFIG_HOME nor HOME set".to_string())?;
    Ok(base.join("autostart"))
}

fn verify_autostart() -> Result<(), String> {
    let dir = autostart_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let path = dir.join(format!("{TAG}.desktop"));

    // Idempotent: nuke any leftover from a prior run.
    let _ = std::fs::remove_file(&path);

    let initial =
        format!("[Desktop Entry]\nType=Application\nName={TAG}\nExec=/bin/true\nHidden=false\n");
    std::fs::write(&path, &initial).map_err(|e| format!("write {}: {e}", path.display()))?;
    println!("    dummy: {}", path.display());

    let result = run_autostart_checks(&path);

    // Always clean up.
    let _ = std::fs::remove_file(&path);
    result
}

fn run_autostart_checks(path: &std::path::Path) -> Result<(), String> {
    let entries = list_entries();
    let entry = entries
        .iter()
        .find(|e| e.path == path)
        .ok_or_else(|| "dummy .desktop not visible to list_entries()".to_string())?;

    if entry.scope != AutostartScope::User {
        return Err(format!("scope: expected User, got {:?}", entry.scope));
    }
    if !entry.enabled {
        return Err("initial state: expected enabled=true".into());
    }

    // Disable.
    let entry = entry.clone();
    let disabled = set_enabled(&entry, false).map_err(|e| format!("set_enabled(false): {e}"))?;
    if disabled.enabled {
        return Err("set_enabled(false) returned enabled=true".into());
    }
    let on_disk = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    if !on_disk.contains("Hidden=true") {
        return Err(format!(
            "after disable: expected Hidden=true, file is:\n{on_disk}"
        ));
    }
    println!("    after disable: Hidden=true present on disk");

    // Enable.
    let enabled = set_enabled(&disabled, true).map_err(|e| format!("set_enabled(true): {e}"))?;
    if !enabled.enabled {
        return Err("set_enabled(true) returned enabled=false".into());
    }
    let on_disk = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    if !on_disk.contains("Hidden=false") {
        return Err(format!(
            "after enable: expected Hidden=false, file is:\n{on_disk}"
        ));
    }
    println!("    after enable: Hidden=false present on disk");

    Ok(())
}

// ------------------------------------------------------------ services ----

fn user_systemd_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| "neither XDG_CONFIG_HOME nor HOME set".to_string())?;
    Ok(base.join("systemd").join("user"))
}

fn systemctl_user(args: &[&str]) -> Result<(bool, String), String> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| format!("spawn systemctl: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

fn verify_services() -> Result<(), String> {
    // Pre-flight: is there a user systemd we can talk to?
    let probe = Command::new("systemctl")
        .args(["--user", "show-environment"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match probe {
        Ok(s) if s.success() => {}
        Ok(s) => return Err(format!("no user systemd available (probe exit: {s})")),
        Err(e) => return Err(format!("systemctl not on PATH: {e}")),
    }

    let unit_name = format!("{TAG}.service");
    let dir = user_systemd_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let path = dir.join(&unit_name);

    // Idempotent: clean up anything from a prior failed run.
    let _ = Command::new("systemctl")
        .args(["--user", "stop", &unit_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("systemctl")
        .args(["--user", "disable", &unit_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = std::fs::remove_file(&path);

    let unit_body = "[Unit]\n\
        Description=Fetcher verification dummy (safe to remove)\n\
        \n\
        [Service]\n\
        Type=simple\n\
        ExecStart=/bin/sleep 999999\n\
        \n\
        [Install]\n\
        WantedBy=default.target\n";
    std::fs::write(&path, unit_body).map_err(|e| format!("write {}: {e}", path.display()))?;
    println!("    dummy unit: {}", path.display());

    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    let result = run_service_checks(&unit_name);

    // Always tear down: stop + disable + remove + reload.
    let _ = Command::new("systemctl")
        .args(["--user", "stop", &unit_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("systemctl")
        .args(["--user", "disable", &unit_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = std::fs::remove_file(&path);
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    result
}

fn run_service_checks(unit_name: &str) -> Result<(), String> {
    // --- Start ---
    service_action(unit_name, ServiceOp::Start, ServiceScope::User)
        .map_err(|e| format!("service_action(Start): {e}"))?;
    thread::sleep(Duration::from_millis(250));
    let (_, active) = systemctl_user(&["is-active", unit_name])?;
    if active != "active" {
        return Err(format!("after Start: expected active=active, got {active}"));
    }
    println!("    after Start: is-active = active");

    // --- Stop ---
    service_action(unit_name, ServiceOp::Stop, ServiceScope::User)
        .map_err(|e| format!("service_action(Stop): {e}"))?;
    thread::sleep(Duration::from_millis(250));
    let (_, active) = systemctl_user(&["is-active", unit_name])?;
    if active == "active" {
        return Err(format!("after Stop: still active ({active})"));
    }
    println!("    after Stop: is-active = {active}");

    // --- Enable ---
    service_action(unit_name, ServiceOp::Enable, ServiceScope::User)
        .map_err(|e| format!("service_action(Enable): {e}"))?;
    thread::sleep(Duration::from_millis(150));
    let (_, en) = systemctl_user(&["is-enabled", unit_name])?;
    if en != "enabled" {
        return Err(format!("after Enable: expected enabled, got {en}"));
    }
    println!("    after Enable: is-enabled = enabled");

    // --- Disable ---
    service_action(unit_name, ServiceOp::Disable, ServiceScope::User)
        .map_err(|e| format!("service_action(Disable): {e}"))?;
    thread::sleep(Duration::from_millis(150));
    let (_, en) = systemctl_user(&["is-enabled", unit_name])?;
    if en == "enabled" {
        return Err(format!("after Disable: still enabled ({en})"));
    }
    println!("    after Disable: is-enabled = {en}");

    Ok(())
}
