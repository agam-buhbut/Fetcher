//! taskmgr-core: OS-touching layer shared by the CLI and GUI frontends.
//!
//! Public surface is intentionally small: a [`Sampler`] you tick once per
//! frame plus a few plain-data snapshot types. No async, no traits, no UI.

pub mod actions;
pub mod performance;
pub mod processes;
pub mod sampler;
pub mod services;
pub mod startup;
pub mod util;

pub use actions::{kill_process, KillSignal};
pub use performance::{CpuStats, DiskStats, MemStats, NetStats};
pub use processes::{ProcessRow, SortColumn, SortOrder, SortState};
pub use sampler::{RefreshKind, Sampler, Snapshot};
pub use services::{ServiceOp, ServiceScope, ServiceUnit};
pub use startup::AutostartEntry;
pub use util::{human_bytes, opt_bytes, truncate};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("permission denied{}", .0.as_deref().map(|s| format!(": {s}")).unwrap_or_default())]
    PermissionDenied(Option<String>),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("dbus: {0}")]
    DBus(String),

    #[error("invalid: {0}")]
    Invalid(String),
}
