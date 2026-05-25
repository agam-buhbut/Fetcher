use std::collections::VecDeque;
use std::time::{Duration, Instant};

use taskmgr_core::services::{list_units, ServiceScope, ServiceUnit};
use taskmgr_core::startup::{list_entries, AutostartEntry};
use taskmgr_core::{processes::sort_in_place, RefreshKind, Sampler, Snapshot, SortState};

pub(crate) const HISTORY_LEN: usize = 120;
pub(crate) const TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Processes,
    Performance,
    Startup,
    Services,
}

impl Tab {
    fn refresh_kind(self) -> RefreshKind {
        match self {
            Self::Processes => RefreshKind::processes_tab(),
            Self::Performance => RefreshKind::performance(),
            Self::Startup | Self::Services => RefreshKind {
                memory: true,
                ..RefreshKind::nothing()
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct App {
    pub(crate) sampler: Sampler,
    pub(crate) tab: Tab,
    pub(crate) snapshot: Snapshot,
    pub(crate) last_tick: Instant,

    pub(crate) cpu_history: VecDeque<f64>,
    pub(crate) mem_history: VecDeque<f64>,
    pub(crate) disk_history: VecDeque<f64>,
    pub(crate) net_history: VecDeque<f64>,

    // processes
    pub(crate) sort: SortState,
    pub(crate) filter: String,

    // startup / services lazy state
    pub(crate) autostart: Vec<AutostartEntry>,
    pub(crate) services: Vec<ServiceUnit>,
    pub(crate) services_scope: ServiceScope,
    pub(crate) startup_dirty: bool,
    pub(crate) services_dirty: bool,

    pub(crate) status: Option<(Instant, String)>,
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            sampler: Sampler::new(),
            tab: Tab::Processes,
            snapshot: Snapshot::default(),
            // Initialise so the first `maybe_tick()` call samples immediately.
            last_tick: Instant::now()
                .checked_sub(TICK)
                .unwrap_or_else(Instant::now),
            cpu_history: VecDeque::with_capacity(HISTORY_LEN),
            mem_history: VecDeque::with_capacity(HISTORY_LEN),
            disk_history: VecDeque::with_capacity(HISTORY_LEN),
            net_history: VecDeque::with_capacity(HISTORY_LEN),
            sort: SortState::default(),
            filter: String::new(),
            autostart: Vec::new(),
            services: Vec::new(),
            services_scope: ServiceScope::User,
            startup_dirty: true,
            services_dirty: true,
            status: None,
        }
    }

    pub(crate) fn maybe_tick(&mut self) {
        // Lazy list loads are NOT throttled — a tab-switch or refresh click
        // should populate within one frame, not up to a TICK later.
        self.refresh_lazy_lists();

        if self.last_tick.elapsed() < TICK {
            return;
        }
        self.last_tick = Instant::now();

        self.snapshot = self.sampler.tick(self.tab.refresh_kind());

        if let Some(c) = &self.snapshot.cpu {
            push(&mut self.cpu_history, f64::from(c.global_usage));
        }
        if let Some(m) = &self.snapshot.memory {
            push(&mut self.mem_history, f64::from(m.used_percent()));
        }
        if let Some(d) = self.snapshot.disk {
            push(
                &mut self.disk_history,
                (d.read_bytes_per_sec + d.write_bytes_per_sec) as f64,
            );
        }
        if let Some(n) = self.snapshot.network {
            push(
                &mut self.net_history,
                (n.rx_bytes_per_sec + n.tx_bytes_per_sec) as f64,
            );
        }

        if let Some(rows) = &mut self.snapshot.processes {
            sort_in_place(rows, self.sort);
        }

        if let Some((t, _)) = self.status {
            if t.elapsed().as_secs() > 4 {
                self.status = None;
            }
        }
    }

    fn refresh_lazy_lists(&mut self) {
        if self.tab == Tab::Startup && self.startup_dirty {
            self.autostart = list_entries();
            self.startup_dirty = false;
        }
        if self.tab == Tab::Services && self.services_dirty {
            match list_units(self.services_scope) {
                Ok(units) => self.services = units,
                Err(e) => {
                    self.set_status(format!("services unavailable: {e}"));
                    self.services.clear();
                }
            }
            self.services.sort_by(|a, b| a.name.cmp(&b.name));
            self.services_dirty = false;
        }
    }

    pub(crate) fn set_status(&mut self, msg: String) {
        self.status = Some((Instant::now(), msg));
    }
}

fn push(buf: &mut VecDeque<f64>, v: f64) {
    if buf.len() == HISTORY_LEN {
        buf.pop_front();
    }
    buf.push_back(v);
}
