//! Proposals for the review screen (API 1.3; feature `proposals`). A plugin proposes a place, a
//! date, tags, a colour label, a face's name, a title or a caption for a photo; the person
//! reviews each on the review screen and only *Accept* writes anything, through Lumen's own
//! paths. Each kind needs its permission (`places.write`, `dates.write`, `tags.write`,
//! `captions.write`, `people.write`) and the whole batch is refused when one proposal isn't
//! granted or isn't valid.


/// The most proposals one host call takes (the host refuses a longer batch whole).
pub const MAX_BATCH: usize = 200;

/// A wall-clock capture time as a date proposal wants it: `YYYY-MM-DDTHH:MM:SS`, no zone.
pub fn date_string(year: u32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

/// Sends `items` to `send` in calls of at most [`MAX_BATCH`], stopping at the first refusal.
pub fn submit_chunks<T>(items: Vec<T>, mut send: impl FnMut(&[T]) -> Result<(), String>) -> Result<(), String> {
    for chunk in items.chunks(MAX_BATCH) {
        send(chunk)?;
    }
    Ok(())
}

#[cfg(all(target_arch = "wasm32", feature = "proposals"))]
mod imp {
    use crate::bindings::lumen::plugin::proposals as p;
    use crate::{FaceName, LatLon, Proposal, Proposed};

    /// One proposal with a 0–1 `confidence` and, optionally, the `reason` the review shows.
    pub fn propose(asset: u64, value: Proposed, confidence: f32, reason: Option<&str>) -> Proposal {
        Proposal { asset, value, confidence, reason: reason.map(str::to_string) }
    }
    pub fn location(lat: f64, lon: f64) -> Proposed {
        Proposed::Location(LatLon { lat, lon })
    }
    /// A name for a face Lumen detected on the photo; never creates a face.
    pub fn person(face: u64, name: &str) -> Proposed {
        Proposed::Person(FaceName { face, name: name.to_string() })
    }

    /// Sends one batch of at most 200 (one bad or ungranted proposal refuses all of it).
    pub fn submit(batch: &[Proposal]) -> Result<(), String> {
        p::submit(batch)
    }
    /// Sends any number, 200 at a time; stops at the first refused call.
    pub fn submit_all(batch: Vec<Proposal>) -> Result<(), String> {
        super::submit_chunks(batch, |chunk| p::submit(chunk))
    }
}

#[cfg(all(target_arch = "wasm32", feature = "proposals"))]
pub use imp::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_batch_goes_in_calls_of_at_most_200() {
        let mut sizes = Vec::new();
        submit_chunks((0..450).collect::<Vec<u32>>(), |chunk| {
            sizes.push(chunk.len());
            Ok(())
        })
        .unwrap();
        assert_eq!(sizes, vec![200, 200, 50]);
    }

    #[test]
    fn the_first_refused_chunk_stops_the_rest() {
        let mut calls = 0;
        let err = submit_chunks((0..450).collect::<Vec<u32>>(), |_| {
            calls += 1;
            if calls == 2 { Err("a label isn't a colour".to_string()) } else { Ok(()) }
        })
        .unwrap_err();
        assert_eq!(calls, 2);
        assert_eq!(err, "a label isn't a colour");
    }

    #[test]
    fn an_empty_batch_calls_nothing() {
        let mut calls = 0;
        submit_chunks(Vec::<u32>::new(), |_| {
            calls += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, 0);
    }

    #[test]
    fn a_date_is_wall_clock_iso_with_no_zone() {
        assert_eq!(date_string(2021, 5, 4, 9, 3, 7), "2021-05-04T09:03:07");
    }
}
