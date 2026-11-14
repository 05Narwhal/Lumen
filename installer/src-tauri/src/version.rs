//! Which version is installed, relative to the one this Setup carries.
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Installed {
    None,
    Older { version: String, location: String },
    Same { version: String, location: String },
    Newer { version: String, location: String },
}

/// `installed` is the registry's `DisplayVersion` (None when missing); `location` is its install
/// folder. Anything unreadable counts as older, so it becomes an Update, never a refusal.
pub fn compare(installed: Option<&str>, location: &str, ours: &str) -> Installed {
    let here = |make: fn(String, String) -> Installed, v: &str| make(v.to_string(), location.to_string());
    let Some(raw) = installed else { return here(older, "") };
    match (parse(raw), parse(ours)) {
        (Some(a), Some(b)) if a == b => here(same, raw),
        (Some(a), Some(b)) if a > b => here(newer, raw),
        (Some(_), None) => here(same, raw),
        _ => here(older, raw),
    }
}

fn older(version: String, location: String) -> Installed {
    Installed::Older { version, location }
}
fn same(version: String, location: String) -> Installed {
    Installed::Same { version, location }
}
fn newer(version: String, location: String) -> Installed {
    Installed::Newer { version, location }
}

/// `26.X.Y-<channel>`: (major, minor, patch, channel rank). No suffix and `-stable` are the
/// stable release, which sorts after beta and alpha of the same number.
fn parse(v: &str) -> Option<(u32, u32, u32, u8)> {
    let (nums, channel) = match v.split_once('-') {
        Some((n, c)) => (n, c),
        None => (v, "stable"),
    };
    let rank = match channel {
        "alpha" => 0,
        "beta" => 1,
        "stable" => 2,
        _ => return None,
    };
    let mut it = nums.split('.');
    let mut next = || it.next()?.parse::<u32>().ok();
    let (a, b, c) = (next()?, next()?, next()?);
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c, rank))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(v: &str, ours: &str) -> &'static str {
        match compare(Some(v), "C:/L", ours) {
            Installed::None => "none",
            Installed::Older { .. } => "older",
            Installed::Same { .. } => "same",
            Installed::Newer { .. } => "newer",
        }
    }

    #[test]
    fn versions_compare_by_channel_and_number() {
        let order = ["26.1.0-alpha", "26.1.0-beta", "26.1.0", "26.1.1-alpha", "26.2.0-alpha"];
        for (i, a) in order.iter().enumerate() {
            for (j, b) in order.iter().enumerate() {
                let want = if i < j { "older" } else if i == j { "same" } else { "newer" };
                assert_eq!(kind(a, b), want, "{a} installed, {b} ours");
            }
        }
        assert_eq!(kind("26.1.0-stable", "26.1.0"), "same");
    }

    #[test]
    fn unreadable_version_counts_as_older() {
        for bad in ["", "garbage", "26.x.0", "26.1", "26.1.0-nightly", "1.2.3.4"] {
            assert_eq!(kind(bad, "26.1.0-alpha"), "older", "{bad:?}");
        }
        assert!(matches!(compare(None, "C:/L", "26.1.0"), Installed::Older { .. }), "an entry without DisplayVersion");
    }
}
