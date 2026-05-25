//! Process row + sort helpers used by both frontends.

#[derive(Debug, Clone)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub user: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub status: ProcessStatus,
    /// Bytes received since the previous snapshot (eBPF). `None` if not measured.
    pub net_rx_per_sec: Option<u64>,
    /// Bytes transmitted since the previous snapshot (eBPF). `None` if not measured.
    pub net_tx_per_sec: Option<u64>,
}

impl ProcessRow {
    /// Case-insensitive substring match across name / pid / user.
    /// Empty needle matches everything.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = needle.to_lowercase();
        self.name.to_lowercase().contains(&needle)
            || self.pid.to_string().contains(&needle)
            || self.user.to_lowercase().contains(&needle)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Sleeping,
    Stopped,
    Zombie,
    Other,
}

impl ProcessStatus {
    #[must_use]
    pub fn short(&self) -> &'static str {
        match self {
            Self::Running => "R",
            Self::Sleeping => "S",
            Self::Stopped => "T",
            Self::Zombie => "Z",
            Self::Other => "?",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Cpu,
    Memory,
    Pid,
    Name,
    NetRx,
    NetTx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// Combined sort state: which column, which direction. Frontends store one of
/// these instead of two parallel fields and call [`SortState::cycle`] when a
/// header is clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortState {
    pub column: SortColumn,
    pub order: SortOrder,
}

impl SortState {
    pub const fn new(column: SortColumn, order: SortOrder) -> Self {
        Self { column, order }
    }

    /// Click on a header: same column flips order, different column resets to
    /// Descending (most-of-something-first is the desktop-task-manager default).
    pub fn cycle(&mut self, column: SortColumn) {
        if self.column == column {
            self.order = match self.order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
        } else {
            self.column = column;
            self.order = SortOrder::Descending;
        }
    }
}

impl Default for SortState {
    fn default() -> Self {
        Self::new(SortColumn::Cpu, SortOrder::Descending)
    }
}

pub fn sort_in_place(rows: &mut [ProcessRow], state: SortState) {
    use std::cmp::Ordering;

    rows.sort_by(|a, b| {
        let ord = match state.column {
            SortColumn::Cpu => a
                .cpu_percent
                .partial_cmp(&b.cpu_percent)
                .unwrap_or(Ordering::Equal),
            SortColumn::Memory => a.memory_bytes.cmp(&b.memory_bytes),
            SortColumn::Pid => a.pid.cmp(&b.pid),
            SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortColumn::NetRx => a
                .net_rx_per_sec
                .unwrap_or(0)
                .cmp(&b.net_rx_per_sec.unwrap_or(0)),
            SortColumn::NetTx => a
                .net_tx_per_sec
                .unwrap_or(0)
                .cmp(&b.net_tx_per_sec.unwrap_or(0)),
        };
        match state.order {
            SortOrder::Ascending => ord,
            SortOrder::Descending => ord.reverse(),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pid: u32, name: &str, user: &str) -> ProcessRow {
        ProcessRow {
            pid,
            parent_pid: None,
            name: name.into(),
            user: user.into(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            status: ProcessStatus::Running,
            net_rx_per_sec: None,
            net_tx_per_sec: None,
        }
    }

    #[test]
    fn matches_is_case_insensitive() {
        let r = row(123, "Firefox", "alice");
        assert!(r.matches("fire"));
        assert!(r.matches("ALICE"));
    }

    #[test]
    fn matches_searches_pid_substring() {
        let r = row(12345, "x", "y");
        assert!(r.matches("234"));
    }

    #[test]
    fn matches_misses_unrelated_needle() {
        let r = row(1, "foo", "bar");
        assert!(!r.matches("zzz"));
    }

    #[test]
    fn matches_empty_needle_matches_anything() {
        let r = row(1, "foo", "bar");
        assert!(r.matches(""));
    }

    #[test]
    fn cycle_same_column_flips_order() {
        let mut s = SortState::new(SortColumn::Cpu, SortOrder::Descending);
        s.cycle(SortColumn::Cpu);
        assert_eq!(s.order, SortOrder::Ascending);
        s.cycle(SortColumn::Cpu);
        assert_eq!(s.order, SortOrder::Descending);
    }

    #[test]
    fn cycle_new_column_resets_to_descending() {
        let mut s = SortState::new(SortColumn::Cpu, SortOrder::Ascending);
        s.cycle(SortColumn::Memory);
        assert_eq!(s.column, SortColumn::Memory);
        assert_eq!(s.order, SortOrder::Descending);
    }
}
