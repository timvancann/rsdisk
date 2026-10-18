//! Human-readable numbers.

/// Bytes in 1000-based units, as Finder shows them: `999 B`, `1.5 KB`, `12.3 GB`.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1000 {
        return format!("{n} B");
    }
    let mut value = n as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_numbers_are_plain_bytes() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
    }

    #[test]
    fn larger_numbers_use_1000_based_units() {
        assert_eq!(bytes(1000), "1.0 KB");
        assert_eq!(bytes(1_500), "1.5 KB");
        assert_eq!(bytes(12_345_678), "12.3 MB");
        assert_eq!(bytes(250_000_000_000), "250.0 GB");
        assert_eq!(bytes(2_000_000_000_000), "2.0 TB");
    }

    #[test]
    fn rounding_never_shows_1000() {
        assert_eq!(bytes(999_999), "1.0 MB");
    }
}
