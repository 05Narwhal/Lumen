//! A deterministic fake library for tests and benchmarks. No files, only rows.

use rusqlite::{params, Connection};

use crate::Result;

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn range(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Inserts `n` assets spread over eight years with mixed aspects, a few videos and some
/// missing capture times. A third carry GPS and a quarter carry a rating, so benchmarks
/// of the structured filters measure real work rather than an empty index. Batched 1000
/// per transaction.
pub fn generate(conn: &Connection, n: usize, seed: u64) -> Result<()> {
    let mut rng = Lcg(seed.wrapping_mul(2654435761).wrapping_add(1));
    let now = 1_758_000_000i64;
    let eight_years = 8 * 365 * 86_400u64;
    let shapes: [(i64, i64); 5] = [(6000, 4000), (4000, 6000), (4032, 3024), (3024, 4032), (1920, 1080)];
    let mut done = 0usize;
    while done < n {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, width, height,
                   orientation, captured_at, imported_at, thumbhash, dominant_color, rating, gps_lat, gps_lon)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            let end = (done + 1000).min(n);
            for i in done..end {
                let (w, h) = shapes[rng.range(5) as usize];
                let is_video = rng.range(20) == 0;
                let captured = if rng.range(50) == 0 { None } else { Some(now - rng.range(eight_years) as i64) };
                let hash = format!("{:016x}{:016x}", rng.next(), i as u64);
                let ext = if is_video { "mp4" } else { "jpg" };
                let rating = if rng.range(4) == 0 { 1 + rng.range(5) as i64 } else { 0 };
                let (lat, lon) = if rng.range(3) == 0 {
                    // Roughly Europe, so clustering has something plausible to work with.
                    (Some(35.0 + rng.range(2000) as f64 / 100.0), Some(-10.0 + rng.range(3000) as f64 / 100.0))
                } else {
                    (None, None)
                };
                stmt.execute(params![
                    hash,
                    format!("2020/01/01/{}_{i}.{ext}", &hash[..8]),
                    format!("IMG_{i}.{ext}"),
                    ext,
                    if is_video { 1 } else { 0 },
                    1_000_000 + rng.range(9_000_000) as i64,
                    w,
                    h,
                    captured,
                    now,
                    vec![0u8; 25],
                    "#808080",
                    rating,
                    lat,
                    lon,
                ])?;
            }
        }
        tx.commit()?;
        done = (done + 1000).min(n);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn generates_rows() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| super::generate(c, 2500, 1)).unwrap();
        let n: i64 = db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 2500);
        let idx = db.with_conn(|c| crate::layout::LayoutIndex::load(c, false)).unwrap();
        let l = idx.compute(1280, 200, 4, Some(crate::layout::GroupBy::Month));
        assert_eq!(l.summary.item_count, 2500);
        assert!(l.summary.sections.len() > 50, "spread across many months: {}", l.summary.sections.len());
    }
}
