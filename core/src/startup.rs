//! XDG autostart entries — list + enable/disable.
//!
//! Reads:
//!   - `~/.config/autostart/*.desktop`        (user-scope, writable)
//!   - `/etc/xdg/autostart/*.desktop`         (system-scope, read-only unless root)
//!
//! Disable strategy for unprivileged user:
//!   - For a user-scope file, set `Hidden=true` in-place.
//!   - For a system-scope file, copy to `~/.config/autostart/` with `Hidden=true`
//!     so the override masks the system entry.
//!
//! Enable: undo the above.

use std::fs;
use std::path::{Path, PathBuf};

use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutostartScope {
    User,
    System,
}

#[derive(Debug, Clone)]
pub struct AutostartEntry {
    pub name: String,
    pub exec: String,
    pub path: PathBuf,
    pub scope: AutostartScope,
    pub enabled: bool,
}

const SYSTEM_AUTOSTART: &str = "/etc/xdg/autostart";

fn user_autostart_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("autostart"))
}

pub fn list_entries() -> Vec<AutostartEntry> {
    let mut out: Vec<AutostartEntry> = Vec::new();

    if let Some(user_dir) = user_autostart_dir() {
        out.extend(read_dir(&user_dir, AutostartScope::User));
    }
    out.extend(read_dir(
        Path::new(SYSTEM_AUTOSTART),
        AutostartScope::System,
    ));

    // If the user has an override of the same basename, drop the system one.
    out.sort_by_key(|a| a.name.to_lowercase());
    let mut seen = std::collections::HashSet::new();
    out.retain(|e| {
        let key = e.path.file_name().map(std::ffi::OsStr::to_owned);
        if let Some(k) = key {
            if e.scope == AutostartScope::System && seen.contains(&k) {
                return false;
            }
            seen.insert(k);
        }
        true
    });

    out
}

fn read_dir(dir: &Path, scope: AutostartScope) -> Vec<AutostartEntry> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "desktop"))
        .filter_map(|e| parse_entry(&e.path(), scope).ok())
        .collect()
}

fn parse_entry(path: &Path, scope: AutostartScope) -> Result<AutostartEntry, CoreError> {
    let raw = fs::read_to_string(path)?;
    let mut name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut exec = String::new();
    let mut hidden = false;
    let mut in_main = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_main = trimmed == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Name=") {
            name = rest.to_string();
        } else if let Some(rest) = trimmed.strip_prefix("Exec=") {
            exec = rest.to_string();
        } else if let Some(rest) = trimmed.strip_prefix("Hidden=") {
            hidden = rest.eq_ignore_ascii_case("true");
        }
    }

    Ok(AutostartEntry {
        name,
        exec,
        path: path.to_path_buf(),
        scope,
        enabled: !hidden,
    })
}

pub fn set_enabled(entry: &AutostartEntry, enabled: bool) -> Result<AutostartEntry, CoreError> {
    let user_dir = user_autostart_dir()
        .ok_or_else(|| CoreError::Invalid("no XDG_CONFIG_HOME or HOME set".into()))?;

    let target_path = match entry.scope {
        AutostartScope::User => entry.path.clone(),
        AutostartScope::System => {
            // Copy/override into user dir, then mutate that copy.
            fs::create_dir_all(&user_dir)?;
            let basename = entry
                .path
                .file_name()
                .ok_or_else(|| CoreError::Invalid("system entry has no filename".into()))?;
            let dst = user_dir.join(basename);
            if !dst.exists() {
                fs::copy(&entry.path, &dst)?;
            }
            dst
        }
    };

    let raw = fs::read_to_string(&target_path)?;
    let new = rewrite_hidden(&raw, !enabled);
    fs::write(&target_path, new)?;

    Ok(AutostartEntry {
        path: target_path,
        scope: AutostartScope::User,
        enabled,
        ..entry.clone()
    })
}

fn rewrite_hidden(raw: &str, hidden: bool) -> String {
    let value = if hidden {
        "Hidden=true"
    } else {
        "Hidden=false"
    };
    let mut out = String::with_capacity(raw.len() + value.len() + 1);
    let mut in_main = false;
    let mut wrote = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_main && !wrote {
                out.push_str(value);
                out.push('\n');
                wrote = true;
            }
            in_main = trimmed == "[Desktop Entry]";
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_main && trimmed.starts_with("Hidden=") {
            out.push_str(value);
            out.push('\n');
            wrote = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if in_main && !wrote {
        out.push_str(value);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_hidden_replaces_existing() {
        let src = "[Desktop Entry]\nName=Foo\nHidden=false\nExec=foo\n";
        let out = rewrite_hidden(src, true);
        assert!(out.contains("Hidden=true"));
        assert!(!out.contains("Hidden=false"));
    }

    #[test]
    fn rewrite_hidden_inserts_when_missing() {
        let src = "[Desktop Entry]\nName=Foo\nExec=foo\n";
        let out = rewrite_hidden(src, true);
        assert!(out.contains("Hidden=true"));
    }

    #[test]
    fn rewrite_hidden_only_main_section() {
        let src = "[Desktop Entry]\nName=Foo\nExec=foo\n[Desktop Action New]\nName=New\n";
        let out = rewrite_hidden(src, true);
        let main_idx = out.find("[Desktop Entry]").unwrap();
        let action_idx = out.find("[Desktop Action New]").unwrap();
        let hidden_idx = out.find("Hidden=true").unwrap();
        assert!(hidden_idx > main_idx && hidden_idx < action_idx);
    }
}
