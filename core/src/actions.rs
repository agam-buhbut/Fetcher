//! Side-effecting actions: kill processes (service ops live in `services`,
//! autostart toggles in `startup`).

use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;

use crate::CoreError;

#[derive(Debug, Clone, Copy)]
pub enum KillSignal {
    /// SIGTERM — polite ask.
    Term,
    /// SIGKILL — force.
    Kill,
}

impl From<KillSignal> for Signal {
    fn from(s: KillSignal) -> Self {
        match s {
            KillSignal::Term => Signal::SIGTERM,
            KillSignal::Kill => Signal::SIGKILL,
        }
    }
}

pub fn kill_process(pid: u32, sig: KillSignal) -> Result<(), CoreError> {
    let pid = Pid::from_raw(pid as i32);
    signal::kill(pid, Signal::from(sig)).map_err(|e| match e {
        nix::errno::Errno::EPERM => CoreError::PermissionDenied(Some(format!("pid {pid}"))),
        nix::errno::Errno::ESRCH => CoreError::NotFound(format!("pid {pid}")),
        other => CoreError::Invalid(other.to_string()),
    })
}
