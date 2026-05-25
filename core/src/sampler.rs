//! Selective-refresh wrapper around [`sysinfo::System`].
//!
//! The frontends call [`Sampler::tick`] once per redraw, asking only for the
//! subsystems the current tab needs. Idle tabs cost nothing.

use std::fs;
use std::time::Instant;

use sysinfo::{Networks, Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind, Users};

use crate::performance::{CpuStats, DiskStats, MemStats, NetStats};
use crate::processes::{ProcessRow, ProcessStatus};

/// Bitset of subsystems to refresh on this tick. The boolean fan-out is
/// deliberate — it's a tiny POD config struct that the frontends construct
/// inline, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct RefreshKind {
    pub cpu: bool,
    pub memory: bool,
    pub processes: bool,
    pub disks: bool,
    pub networks: bool,
}

impl RefreshKind {
    pub const fn nothing() -> Self {
        Self {
            cpu: false,
            memory: false,
            processes: false,
            disks: false,
            networks: false,
        }
    }
    pub const fn performance() -> Self {
        Self {
            cpu: true,
            memory: true,
            processes: false,
            disks: true,
            networks: true,
        }
    }
    pub const fn processes() -> Self {
        Self {
            cpu: true,
            memory: true,
            processes: true,
            disks: false,
            networks: false,
        }
    }
    /// What the Processes tab needs each tick: live CPU%, live mem usage,
    /// and the process list itself.
    pub const fn processes_tab() -> Self {
        Self::processes()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu: Option<CpuStats>,
    pub memory: Option<MemStats>,
    pub disk: Option<DiskStats>,
    pub network: Option<NetStats>,
    pub processes: Option<Vec<ProcessRow>>,
}

#[derive(Debug)]
pub struct Sampler {
    system: System,
    networks: Networks,
    users: Users,
    last_tick: Option<Instant>,
    last_diskstats: Option<(Instant, DiskCounters)>,
}

#[derive(Debug, Clone, Copy, Default)]
struct DiskCounters {
    read_bytes: u64,
    write_bytes: u64,
}

impl Sampler {
    pub fn new() -> Self {
        let mut system = System::new();
        // Prime CPU values so the first usage reading isn't 0/garbage.
        system.refresh_cpu_usage();
        Self {
            system,
            networks: Networks::new_with_refreshed_list(),
            users: Users::new_with_refreshed_list(),
            last_tick: None,
            last_diskstats: None,
        }
    }

    pub fn tick(&mut self, what: RefreshKind) -> Snapshot {
        let now = Instant::now();
        let elapsed_secs = self
            .last_tick
            .map(|t| (now - t).as_secs_f64())
            .filter(|s| *s > 0.0)
            .unwrap_or(1.0);
        self.last_tick = Some(now);

        let mut snap = Snapshot::default();

        if what.cpu {
            self.system.refresh_cpu_usage();
            let per_core: Vec<f32> = self
                .system
                .cpus()
                .iter()
                .map(sysinfo::Cpu::cpu_usage)
                .collect();
            let global_usage = self.system.global_cpu_usage();
            snap.cpu = Some(CpuStats {
                global_usage,
                per_core,
            });
        }

        if what.memory {
            self.system.refresh_memory();
            snap.memory = Some(MemStats {
                total_bytes: self.system.total_memory(),
                used_bytes: self.system.used_memory(),
                available_bytes: self.system.available_memory(),
                swap_total_bytes: self.system.total_swap(),
                swap_used_bytes: self.system.used_swap(),
            });
        }

        if what.disks {
            snap.disk = Some(self.sample_disk_throughput(now));
        }

        if what.networks {
            self.networks.refresh();
            let mut rx_total: u64 = 0;
            let mut tx_total: u64 = 0;
            for (_name, data) in &self.networks {
                rx_total = rx_total.saturating_add(data.received());
                tx_total = tx_total.saturating_add(data.transmitted());
            }
            snap.network = Some(NetStats {
                rx_bytes_per_sec: (rx_total as f64 / elapsed_secs) as u64,
                tx_bytes_per_sec: (tx_total as f64 / elapsed_secs) as u64,
            });
        }

        if what.processes {
            self.system.refresh_processes_specifics(
                ProcessesToUpdate::All,
                true,
                ProcessRefreshKind::new()
                    .with_cpu()
                    .with_memory()
                    .with_user(UpdateKind::OnlyIfNotSet),
            );
            let users = &self.users;
            let rows: Vec<ProcessRow> = self
                .system
                .processes()
                .iter()
                .map(|(pid, p)| ProcessRow {
                    pid: pid.as_u32(),
                    parent_pid: p.parent().map(Pid::as_u32),
                    name: p.name().to_string_lossy().into_owned(),
                    user: p
                        .user_id()
                        .and_then(|uid| users.get_user_by_id(uid))
                        .map_or_else(|| "-".to_string(), |u| u.name().to_string()),
                    cpu_percent: p.cpu_usage(),
                    memory_bytes: p.memory(),
                    status: match p.status() {
                        sysinfo::ProcessStatus::Run => ProcessStatus::Running,
                        sysinfo::ProcessStatus::Sleep | sysinfo::ProcessStatus::Idle => {
                            ProcessStatus::Sleeping
                        }
                        sysinfo::ProcessStatus::Stop => ProcessStatus::Stopped,
                        sysinfo::ProcessStatus::Zombie | sysinfo::ProcessStatus::Dead => {
                            ProcessStatus::Zombie
                        }
                        _ => ProcessStatus::Other,
                    },
                    net_rx_per_sec: None,
                    net_tx_per_sec: None,
                })
                .collect();
            snap.processes = Some(rows);
        }

        snap
    }

    /// Disk throughput from `/proc/diskstats`. We compute deltas across ticks;
    /// the first tick returns zeros.
    fn sample_disk_throughput(&mut self, now: Instant) -> DiskStats {
        let counters = read_diskstats().unwrap_or_default();
        let stats = match self.last_diskstats {
            Some((prev_t, prev)) => {
                let dt = (now - prev_t).as_secs_f64().max(0.001);
                DiskStats {
                    read_bytes_per_sec: ((counters.read_bytes.saturating_sub(prev.read_bytes))
                        as f64
                        / dt) as u64,
                    write_bytes_per_sec: ((counters.write_bytes.saturating_sub(prev.write_bytes))
                        as f64
                        / dt) as u64,
                }
            }
            None => DiskStats::default(),
        };
        self.last_diskstats = Some((now, counters));
        stats
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

/// Read whole-disk byte counters from `/sys/block/*/stat`. Each subdir of
/// `/sys/block` is a whole disk by kernel convention, so we never
/// double-count partitions. Sector size is the kernel constant 512 bytes.
fn read_diskstats() -> Option<DiskCounters> {
    let mut counters = DiskCounters::default();
    let entries = fs::read_dir("/sys/block").ok()?;
    for e in entries.flatten() {
        let name = e.file_name();
        let name_s = name.to_string_lossy();
        // Skip RAM disks and loop devices; they inflate totals with no real I/O significance.
        if name_s.starts_with("ram") || name_s.starts_with("loop") {
            continue;
        }
        let stat_path = e.path().join("stat");
        let Ok(raw) = fs::read_to_string(&stat_path) else {
            continue;
        };
        // Format (one line, whitespace-separated):
        //   reads merges sectors_read ms_read writes wmerges sectors_written ...
        let fields: Vec<&str> = raw.split_whitespace().collect();
        if fields.len() < 7 {
            continue;
        }
        let sectors_read: u64 = fields[2].parse().unwrap_or(0);
        let sectors_written: u64 = fields[6].parse().unwrap_or(0);
        counters.read_bytes = counters.read_bytes.saturating_add(sectors_read * 512);
        counters.write_bytes = counters.write_bytes.saturating_add(sectors_written * 512);
    }
    Some(counters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampler_returns_memory_snapshot() {
        let mut s = Sampler::new();
        let snap = s.tick(RefreshKind {
            memory: true,
            ..RefreshKind::nothing()
        });
        let mem = snap.memory.expect("memory requested");
        assert!(mem.total_bytes > 0, "host should have nonzero memory");
        assert!(mem.used_bytes <= mem.total_bytes);
    }

    #[test]
    fn sampler_lists_own_pid() {
        let mut s = Sampler::new();
        let snap = s.tick(RefreshKind::processes());
        let rows = snap.processes.expect("processes requested");
        let me = std::process::id();
        assert!(rows.iter().any(|r| r.pid == me), "own pid should appear");
    }

    #[test]
    fn cpu_snapshot_has_per_core_entries() {
        let mut s = Sampler::new();
        // First tick primes counters; second produces real numbers.
        s.tick(RefreshKind {
            cpu: true,
            ..RefreshKind::nothing()
        });
        std::thread::sleep(std::time::Duration::from_millis(250));
        let snap = s.tick(RefreshKind {
            cpu: true,
            ..RefreshKind::nothing()
        });
        let cpu = snap.cpu.expect("cpu requested");
        assert!(!cpu.per_core.is_empty());
    }

    #[test]
    fn diskstats_reads_without_panic() {
        // /sys/block always exists on Linux; we just want to confirm parsing
        // doesn't trip up on this host's actual layout.
        let _ = read_diskstats();
    }
}
