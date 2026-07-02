use std::collections::VecDeque;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::widgets::TableState;
use taskmgr_core::services::{list_units, service_action, ServiceOp, ServiceScope, ServiceUnit};
use taskmgr_core::startup::{list_entries, set_enabled, AutostartEntry};
use taskmgr_core::{
    kill_process, processes::sort_in_place, KillSignal, ProcessRow, RefreshKind, Sampler, Snapshot,
    SortColumn, SortState,
};

const HISTORY_LEN: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Processes,
    Performance,
    Startup,
    Services,
}

impl Tab {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Processes => Self::Performance,
            Self::Performance => Self::Startup,
            Self::Startup => Self::Services,
            Self::Services => Self::Processes,
        }
    }
    pub(crate) fn prev(self) -> Self {
        match self {
            Self::Processes => Self::Services,
            Self::Performance => Self::Processes,
            Self::Startup => Self::Performance,
            Self::Services => Self::Startup,
        }
    }
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Processes => "Processes",
            Self::Performance => "Performance",
            Self::Startup => "Startup",
            Self::Services => "Services",
        }
    }

    /// CPU/mem/disk/net are a handful of /proc reads — sample them every
    /// tick on every tab so the Performance graphs never have time gaps.
    /// Only the expensive process-table refresh is gated to its tab.
    fn refresh_kind(self) -> RefreshKind {
        RefreshKind {
            cpu: true,
            memory: true,
            disks: true,
            networks: true,
            processes: self == Self::Processes,
        }
    }
}

#[derive(Debug)]
pub(crate) struct App {
    sampler: Sampler,
    pub(crate) tab: Tab,
    pub(crate) snapshot: Snapshot,

    // History buffers for the Performance tab.
    pub(crate) cpu_history: VecDeque<u64>,
    pub(crate) mem_history: VecDeque<u64>,
    pub(crate) disk_history: VecDeque<u64>,
    pub(crate) net_history: VecDeque<u64>,

    // Processes tab. Selection is tracked by PID (the index is re-derived
    // after every re-sort) so the highlight — and `x` — stays on the same
    // process while the list churns underneath.
    pub(crate) proc_selected: usize,
    pub(crate) proc_selected_pid: Option<u32>,
    pub(crate) sort: SortState,
    pub(crate) filter: String,
    pub(crate) filter_active: bool,

    // Table widget state persists across frames so the scroll offset is
    // stable instead of resetting every draw.
    pub(crate) proc_table: TableState,
    pub(crate) startup_table: TableState,
    pub(crate) services_table: TableState,

    // Startup tab.
    pub(crate) autostart: Vec<AutostartEntry>,
    pub(crate) startup_selected: usize,
    pub(crate) startup_dirty: bool,

    // Services tab.
    pub(crate) services: Vec<ServiceUnit>,
    pub(crate) services_selected: usize,
    pub(crate) services_scope: ServiceScope,
    pub(crate) services_dirty: bool,

    pub(crate) status: Option<(Instant, String)>,
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            sampler: Sampler::new(),
            tab: Tab::Processes,
            snapshot: Snapshot::default(),
            cpu_history: VecDeque::with_capacity(HISTORY_LEN),
            mem_history: VecDeque::with_capacity(HISTORY_LEN),
            disk_history: VecDeque::with_capacity(HISTORY_LEN),
            net_history: VecDeque::with_capacity(HISTORY_LEN),
            proc_selected: 0,
            proc_selected_pid: None,
            sort: SortState::default(),
            filter: String::new(),
            filter_active: false,
            proc_table: TableState::default(),
            startup_table: TableState::default(),
            services_table: TableState::default(),
            autostart: Vec::new(),
            startup_selected: 0,
            startup_dirty: true,
            services: Vec::new(),
            services_selected: 0,
            services_scope: ServiceScope::User,
            services_dirty: true,
            status: None,
        }
    }

    pub(crate) fn tick(&mut self) {
        let mut snap = self.sampler.tick(self.tab.refresh_kind());
        // Keep the last process list alive while on other tabs so switching
        // back to Processes never shows a blank table for a tick.
        if snap.processes.is_none() {
            snap.processes = self.snapshot.processes.take();
        }
        self.snapshot = snap;

        // Update Performance history regardless of tab so the graphs aren't blank
        // the moment the user switches.
        if let Some(cpu) = &self.snapshot.cpu {
            push_capped(&mut self.cpu_history, cpu.global_usage as u64);
        }
        if let Some(mem) = &self.snapshot.memory {
            push_capped(&mut self.mem_history, mem.used_percent() as u64);
        }
        if let Some(d) = self.snapshot.disk {
            push_capped(
                &mut self.disk_history,
                d.read_bytes_per_sec.saturating_add(d.write_bytes_per_sec),
            );
        }
        if let Some(n) = self.snapshot.network {
            push_capped(
                &mut self.net_history,
                n.rx_bytes_per_sec.saturating_add(n.tx_bytes_per_sec),
            );
        }

        // Apply current sort to processes, then re-locate the selected PID.
        if let Some(rows) = &mut self.snapshot.processes {
            sort_in_place(rows, self.sort);
        }
        self.resync_proc_selection();

        if let Some((t, _)) = self.status {
            if t.elapsed().as_secs() > 4 {
                self.status = None;
            }
        }
    }

    /// Load the Startup / Services lists if marked dirty. Deliberately
    /// decoupled from the throttled sampler tick so a tab-switch or refresh
    /// shows fresh data on the *next* frame, not up to a TICK later.
    pub(crate) fn refresh_lazy_lists(&mut self) {
        if self.tab == Tab::Startup && self.startup_dirty {
            self.autostart = list_entries();
            self.startup_dirty = false;
            self.startup_selected = self
                .startup_selected
                .min(self.autostart.len().saturating_sub(1));
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
            self.services_selected = self
                .services_selected
                .min(self.services.len().saturating_sub(1));
        }
    }

    pub(crate) fn move_selection(&mut self, delta: i32) {
        let len = self.selection_len();
        if len == 0 {
            return;
        }
        let cur = self.selection_idx();
        self.set_selection((cur as i32 + delta).clamp(0, len as i32 - 1) as usize);
    }

    pub(crate) fn jump_to(&mut self, idx: i32) {
        let len = self.selection_len();
        if len == 0 {
            return;
        }
        self.set_selection(idx.clamp(0, len as i32 - 1) as usize);
    }

    fn selection_idx(&self) -> usize {
        match self.tab {
            Tab::Processes => self.proc_selected,
            Tab::Startup => self.startup_selected,
            Tab::Services => self.services_selected,
            Tab::Performance => 0,
        }
    }

    fn selection_len(&self) -> usize {
        match self.tab {
            Tab::Processes => self.filtered_processes().len(),
            Tab::Startup => self.autostart.len(),
            Tab::Services => self.services.len(),
            Tab::Performance => 0,
        }
    }

    fn set_selection(&mut self, idx: usize) {
        match self.tab {
            Tab::Processes => {
                let pid = self.filtered_processes().get(idx).map(|r| r.pid);
                self.proc_selected = idx;
                self.proc_selected_pid = pid;
            }
            Tab::Startup => self.startup_selected = idx,
            Tab::Services => self.services_selected = idx,
            Tab::Performance => {}
        }
    }

    /// Re-derive `proc_selected` from the tracked PID after anything that
    /// reorders or shrinks the visible list (tick, sort change, filter edit).
    /// If the process is gone, stay at the same spot and adopt that row.
    pub(crate) fn resync_proc_selection(&mut self) {
        let (len, found, pid_here) = {
            let rows = self.filtered_processes();
            let found = self
                .proc_selected_pid
                .and_then(|pid| rows.iter().position(|r| r.pid == pid));
            let clamped = self.proc_selected.min(rows.len().saturating_sub(1));
            (rows.len(), found, rows.get(clamped).map(|r| r.pid))
        };
        if len == 0 {
            self.proc_selected = 0;
            self.proc_selected_pid = None;
            return;
        }
        if let Some(idx) = found {
            self.proc_selected = idx;
        } else {
            self.proc_selected = self.proc_selected.min(len - 1);
            self.proc_selected_pid = pid_here;
        }
    }

    pub(crate) fn filtered_processes(&self) -> Vec<&ProcessRow> {
        let Some(rows) = &self.snapshot.processes else {
            return Vec::new();
        };
        rows.iter().filter(|r| r.matches(&self.filter)).collect()
    }

    pub(crate) fn handle_processes_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('/') => {
                self.filter_active = true;
                self.filter.clear();
                self.resync_proc_selection();
            }
            // 'k' is reserved as the global vim-style "move up" binding, so
            // kill uses 'x' (term) / 'X' (kill -9). Plain Delete works too.
            KeyCode::Char('x') | KeyCode::Delete => self.kill_selected(KillSignal::Term),
            KeyCode::Char('X') => self.kill_selected(KillSignal::Kill),
            KeyCode::Char('s') => self.cycle_sort(SortColumn::Cpu),
            KeyCode::Char('m') => self.cycle_sort(SortColumn::Memory),
            KeyCode::Char('p') => self.cycle_sort(SortColumn::Pid),
            KeyCode::Char('n') => self.cycle_sort(SortColumn::Name),
            KeyCode::Char('d') => self.cycle_sort(SortColumn::DiskRead),
            KeyCode::Char('w') => self.cycle_sort(SortColumn::DiskWrite),
            _ => {}
        }
    }

    /// Sort changes take effect on the next frame, not the next 1s tick.
    fn cycle_sort(&mut self, column: SortColumn) {
        self.sort.cycle(column);
        if let Some(rows) = &mut self.snapshot.processes {
            sort_in_place(rows, self.sort);
        }
        self.resync_proc_selection();
    }

    fn kill_selected(&mut self, sig: KillSignal) {
        let rows = self.filtered_processes();
        if rows.is_empty() {
            return;
        }
        // Clamp to the (possibly just-filtered) row count so the action
        // matches the visually-highlighted row.
        let idx = self.proc_selected.min(rows.len() - 1);
        let target = rows[idx];
        let pid = target.pid;
        let name = target.name.clone();
        match kill_process(pid, sig) {
            Ok(()) => self.set_status(format!("sent {sig:?} to {name} ({pid})")),
            Err(e) => self.set_status(format!("kill {pid} failed: {e}")),
        }
    }

    pub(crate) fn handle_startup_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(' ') => self.toggle_selected_autostart(),
            KeyCode::Char('r') => {
                self.startup_dirty = true;
                self.set_status("refreshing startup".into());
            }
            _ => {}
        }
    }

    fn toggle_selected_autostart(&mut self) {
        let Some(entry) = self.autostart.get(self.startup_selected).cloned() else {
            return;
        };
        match set_enabled(&entry, !entry.enabled) {
            Ok(updated) => {
                self.autostart[self.startup_selected] = updated;
                self.set_status(format!(
                    "{} {}",
                    if entry.enabled { "disabled" } else { "enabled" },
                    entry.name
                ));
            }
            Err(e) => self.set_status(format!("autostart toggle failed: {e}")),
        }
    }

    pub(crate) fn handle_services_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('s') => self.do_service(ServiceOp::Start),
            KeyCode::Char('S') => self.do_service(ServiceOp::Stop),
            KeyCode::Char('R') => self.do_service(ServiceOp::Restart),
            KeyCode::Char('e') => self.do_service(ServiceOp::Enable),
            KeyCode::Char('E') => self.do_service(ServiceOp::Disable),
            KeyCode::Char('u') => {
                self.services_scope = match self.services_scope {
                    ServiceScope::User => ServiceScope::System,
                    ServiceScope::System => ServiceScope::User,
                };
                self.services_dirty = true;
                self.services_selected = 0;
            }
            KeyCode::Char('r') => {
                self.services_dirty = true;
                self.set_status("refreshing services".into());
            }
            _ => {}
        }
    }

    fn do_service(&mut self, op: ServiceOp) {
        let Some(unit) = self.services.get(self.services_selected).cloned() else {
            return;
        };
        match service_action(&unit.name, op, self.services_scope) {
            Ok(()) => {
                self.set_status(format!("{op:?} {}", unit.name));
                self.services_dirty = true;
            }
            Err(e) => self.set_status(format!("{op:?} {} failed: {e}", unit.name)),
        }
    }

    pub(crate) fn set_status(&mut self, msg: String) {
        self.status = Some((Instant::now(), msg));
    }
}

fn push_capped(buf: &mut VecDeque<u64>, v: u64) {
    if buf.len() == HISTORY_LEN {
        buf.pop_front();
    }
    buf.push_back(v);
}
