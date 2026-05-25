//! Small formatting helpers shared by both frontends.

/// Format a byte count using binary (KiB-style) units, no trailing space-pad.
/// `1023 → "1023 B"`, `1024 → "1.0 K"`, `2_500_000 → "2.4 M"`.
pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

/// Truncate to `max` Unicode chars; appends `…` if anything was cut.
/// `truncate("hello", 10) → "hello"`, `truncate("hello world", 5) → "hell…"`.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Format an optional byte count, rendering `None` as `—`.
pub fn opt_bytes(v: Option<u64>) -> String {
    match v {
        Some(b) => human_bytes(b),
        None => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_boundary_values() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1024), "1.0 K");
        assert_eq!(human_bytes(1024 * 1024), "1.0 M");
    }

    #[test]
    fn human_bytes_petabyte_does_not_overflow_unit_table() {
        // 1 EiB exceeds the table; should clamp at "P".
        let big = 1_152_921_504_606_846_976u64; // 1 EiB
        assert!(human_bytes(big).ends_with(" P"));
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate("hello world", 5), "hell…");
    }

    #[test]
    fn opt_bytes_renders_dash_for_none() {
        assert_eq!(opt_bytes(None), "—");
        assert_eq!(opt_bytes(Some(1024)), "1.0 K");
    }
}
