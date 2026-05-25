//! Plain-data snapshot types for the Performance tab.

#[derive(Debug, Clone, Default)]
pub struct CpuStats {
    /// Aggregate CPU usage in percent (0..=100).
    pub global_usage: f32,
    /// Per-logical-core usage in percent.
    pub per_core: Vec<f32>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MemStats {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

impl MemStats {
    pub fn used_percent(&self) -> f32 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.used_bytes as f64 / self.total_bytes as f64 * 100.0) as f32
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DiskStats {
    /// Bytes read across all disks since the previous snapshot.
    pub read_bytes_per_sec: u64,
    /// Bytes written across all disks since the previous snapshot.
    pub write_bytes_per_sec: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NetStats {
    /// Bytes received across all interfaces since the previous snapshot.
    pub rx_bytes_per_sec: u64,
    /// Bytes transmitted across all interfaces since the previous snapshot.
    pub tx_bytes_per_sec: u64,
}
