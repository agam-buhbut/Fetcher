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
    /// Disk read rate in bytes/sec, from `/proc/<pid>/io` deltas.
    /// 0 on the first sample and for other users' processes when not root
    /// (the kernel hides their counters) — indistinguishable from a true 0.
    pub disk_read_per_sec: u64,
    /// Disk write rate in bytes/sec. Same caveats as `disk_read_per_sec`.
    pub disk_write_per_sec: u64,
}

impl ProcessRow {
    /// Case-insensitive substring match across name / pid / user.
    /// Empty needle matches everything. Case folding is ASCII-only — this
    /// runs per row per keystroke, and process/user names are ASCII in
    /// practice, so we skip Unicode folding to stay allocation-free.
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        contains_ignore_ascii_case(&self.name, needle)
            || self.pid.to_string().contains(needle)
            || contains_ignore_ascii_case(&self.user, needle)
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
    DiskRead,
    DiskWrite,
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

    // PID tiebreak: the source is a HashMap, so without it equal-key rows
    // (e.g. the many 0.0% CPU ones) reshuffle on every tick.
    rows.sort_unstable_by(|a, b| {
        let ord = match state.column {
            SortColumn::Cpu => a
                .cpu_percent
                .partial_cmp(&b.cpu_percent)
                .unwrap_or(Ordering::Equal),
            SortColumn::Memory => a.memory_bytes.cmp(&b.memory_bytes),
            SortColumn::Pid => a.pid.cmp(&b.pid),
            SortColumn::Name => cmp_ignore_ascii_case(&a.name, &b.name),
            SortColumn::DiskRead => a.disk_read_per_sec.cmp(&b.disk_read_per_sec),
            SortColumn::DiskWrite => a.disk_write_per_sec.cmp(&b.disk_write_per_sec),
        };
        let ord = match state.order {
            SortOrder::Ascending => ord,
            SortOrder::Descending => ord.reverse(),
        };
        ord.then_with(|| a.pid.cmp(&b.pid))
    });
}

/// ASCII-case-insensitive substring search without allocating lowercased
/// copies (the `to_lowercase` version allocated 2 Strings per row per frame).
fn contains_ignore_ascii_case(hay: &str, needle: &str) -> bool {
    let (hay, needle) = (hay.as_bytes(), needle.as_bytes());
    if needle.len() > hay.len() {
        return false;
    }
    hay.windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

/// Allocation-free ASCII-case-insensitive ordering (name sort previously
/// allocated two lowercased Strings per comparison, O(n log n) per tick).
fn cmp_ignore_ascii_case(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.bytes().map(|c| c.to_ascii_lowercase());
    let b = b.bytes().map(|c| c.to_ascii_lowercase());
    a.cmp(b)
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
            disk_read_per_sec: 0,
            disk_write_per_sec: 0,
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
    fn sort_equal_keys_is_deterministic_by_pid() {
        // All rows tie on CPU (0.0); order must fall back to PID, not input order.
        let mut a = vec![row(30, "c", "u"), row(10, "a", "u"), row(20, "b", "u")];
        let mut b = vec![row(10, "a", "u"), row(20, "b", "u"), row(30, "c", "u")];
        let state = SortState::new(SortColumn::Cpu, SortOrder::Descending);
        sort_in_place(&mut a, state);
        sort_in_place(&mut b, state);
        let pids: Vec<u32> = a.iter().map(|r| r.pid).collect();
        assert_eq!(pids, b.iter().map(|r| r.pid).collect::<Vec<_>>());
        assert_eq!(pids, vec![10, 20, 30]);
    }

    #[test]
    fn sort_by_disk_read_descending() {
        let mut rows = vec![row(1, "a", "u"), row(2, "b", "u"), row(3, "c", "u")];
        rows[0].disk_read_per_sec = 10;
        rows[2].disk_read_per_sec = 999;
        sort_in_place(
            &mut rows,
            SortState::new(SortColumn::DiskRead, SortOrder::Descending),
        );
        let pids: Vec<u32> = rows.iter().map(|r| r.pid).collect();
        assert_eq!(pids, vec![3, 1, 2]);
    }

    #[test]
    fn sort_by_name_ignores_ascii_case() {
        let mut rows = vec![
            row(1, "zsh", "u"),
            row(2, "Bash", "u"),
            row(3, "alpha", "u"),
        ];
        sort_in_place(
            &mut rows,
            SortState::new(SortColumn::Name, SortOrder::Ascending),
        );
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Bash", "zsh"]);
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
