/// Parse an HTTP `Range` header into an inclusive `(start, end)` for a body of `total` bytes.
///
/// Supports `bytes=a-b`, `bytes=a-` and `bytes=-n`. Returns `None` when the range
/// is malformed or unsatisfiable; callers answer `416` in that case.
pub fn parse(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            if n == 0 {
                return None;
            }
            (total.saturating_sub(n), total - 1)
        }
        (s, "") => (s.parse().ok()?, total - 1),
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(total - 1)),
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parses_forms() {
        assert_eq!(parse("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse("bytes=0-5000", 1000), Some((0, 999)), "end is clamped");
    }

    #[test]
    fn rejects_bad_ranges() {
        assert_eq!(parse("bytes=1000-", 1000), None);
        assert_eq!(parse("bytes=5-2", 1000), None);
        assert_eq!(parse("items=0-1", 1000), None);
        assert_eq!(parse("bytes=0-1", 0), None);
    }
}
