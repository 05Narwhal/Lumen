//! Duplicates on a real catalogue (Round 4D). Ignored by default; point it at a *copy* of a
//! library's database (opening it runs the migrations):
//!
//! ```sh
//! LUMEN_DB=/tmp/copy/library.db LUMEN_DUP_OUT=/tmp/sets.json \
//!   cargo test --release -p photos-core --test duplicates_real -- --ignored --nocapture
//! ```
//!
//! `LUMEN_DUP_OUT` receives every set with its members' content hashes, for a contact sheet.

use std::path::PathBuf;
use std::time::Instant;

use photos_core::duplicates::{self, Likeness};
use photos_core::Db;

#[test]
#[ignore = "needs LUMEN_DB, a copy of a library database"]
fn finds_duplicates_in_a_real_library() {
    let Some(path) = std::env::var_os("LUMEN_DB") else {
        println!("LUMEN_DB not set");
        return;
    };
    let db = Db::open(&PathBuf::from(path)).expect("open the copy");
    db.with_conn(|c| {
        let t = Instant::now();
        let sets = duplicates::find_sets(c)?;
        let ms = t.elapsed().as_millis();
        let exact = sets.iter().filter(|s| s.likeness == Likeness::Exact).count();
        let removable: usize = sets.iter().map(|s| s.members.len() - 1).sum();
        println!("{} sets ({exact} exact, {} similar), {removable} photos could go, in {ms} ms", sets.len(), sets.len() - exact);
        let mut sizes = std::collections::BTreeMap::new();
        for s in &sets {
            *sizes.entry(s.members.len()).or_insert(0) += 1;
        }
        println!("set sizes: {sizes:?}");
        if let Some(out) = std::env::var_os("LUMEN_DUP_OUT") {
            let mut hash = c.prepare("SELECT content_hash, filename FROM assets WHERE id = ?1")?;
            let mut json = Vec::new();
            for s in &sets {
                let members: Vec<serde_json::Value> = s
                    .members
                    .iter()
                    .map(|m| {
                        let (h, f): (String, String) = hash.query_row([m.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
                        serde_json::json!({ "id": m.id, "hash": h, "filename": f, "member": m })
                    })
                    .collect();
                json.push(serde_json::json!({ "key": s.key, "likeness": s.likeness, "keep": s.keep, "members": members }));
            }
            std::fs::write(out, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        }
        Ok(())
    })
    .unwrap();
}
