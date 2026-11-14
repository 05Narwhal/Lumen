//! Embedding backfill (ML design §5): every visible photo gets a CLIP vector from its 256 px
//! thumbnail, a batch at a time on the Maintenance lane, the way `backfill_quality` fills
//! sharpness. With the model missing or smart features off this does nothing.

use std::collections::HashSet;

use photos_core::embedding;
use photos_media::DerivativeStore;
use photos_ml::clip::Rgb;
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// Photos per run: the model is several times faster per photo in a batch of 16.
const BATCH: usize = 16;

pub fn rgb_of(img: &image::DynamicImage) -> (Vec<u8>, u32, u32) {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    (rgb.into_raw(), w, h)
}

/// Embeds up to one batch. Returns how many were stored, and how many rows were looked at,
/// so the caller can tell "done" from "every remaining thumbnail is unreadable".
fn backfill(app: &AppHandle, skip: &mut HashSet<i64>) -> (usize, usize) {
    let state = app.state::<AppState>();
    let Ok(lib) = state.library() else { return (0, 0) };
    let Some(entry) = photos_ml::manifest::manifest().by_role("image_embedding") else { return (0, 0) };
    let Ok(rows) = lib.db.with_conn(|c| embedding::missing(c, &entry.id, BATCH + skip.len())) else { return (0, 0) };
    let rows: Vec<(i64, String)> = rows.into_iter().filter(|(id, _)| !skip.contains(id)).take(BATCH).collect();
    if rows.is_empty() {
        return (0, 0);
    }
    let store = DerivativeStore::new(&lib.root);
    let mut pixels: Vec<(i64, Vec<u8>, u32, u32)> = Vec::with_capacity(rows.len());
    for (id, hash) in &rows {
        match image::open(store.thumb_path(hash, 256)) {
            Ok(img) => {
                let (rgb, w, h) = rgb_of(&img);
                pixels.push((*id, rgb, w, h));
            }
            // Not rendered yet, or unreadable: leave it for a later run.
            Err(_) => {
                skip.insert(*id);
            }
        }
    }
    let batch: Vec<Rgb<'_>> = pixels.iter().map(|(_, d, w, h)| Rgb { data: d, width: *w, height: *h }).collect();
    let Some(result) = crate::models::with_encoder(app, |enc| enc.embed_batch(&batch)) else { return (0, rows.len()) };
    let out = match result {
        Ok(out) => out,
        Err(e) => {
            tracing::warn!("embedding: {e}");
            for (id, ..) in &pixels {
                skip.insert(*id);
            }
            return (0, rows.len());
        }
    };
    let protos = photos_ml::prototypes();
    let mut stored = 0;
    for ((id, ..), (vec, aesthetic)) in pixels.iter().zip(out) {
        let tags = embedding::scene_tags(&vec, &protos);
        let saved = lib.db.with_conn(|c| {
            embedding::store(c, *id, &entry.id, &vec, aesthetic)?;
            embedding::set_scene_tags(c, *id, &tags)
        });
        if saved.is_ok() {
            stored += 1;
        }
    }
    (stored, rows.len())
}

/// Embed everything missing, on the Maintenance lane (coalesced), then regenerate memories
/// so the highlights see the new embeddings.
pub fn schedule(app: &AppHandle) {
    if !crate::models::enabled(app) {
        return;
    }
    let a = app.clone();
    app.state::<AppState>().jobs.submit("maintenance:embeddings", photos_jobs::Lane::Maintenance, move |cancel| {
        let mut skip = HashSet::new();
        let (mut total, started) = (0usize, std::time::Instant::now());
        loop {
            let (stored, seen) = backfill(&a, &mut skip);
            total += stored;
            if seen == 0 || cancel.is_cancelled() {
                break;
            }
        }
        // Embeddings stored before scene tags existed get their tags from the vector alone.
        let protos = photos_ml::prototypes();
        loop {
            let state = a.state::<AppState>();
            let Ok(lib) = state.library() else { break };
            let Ok(batch) = lib.db.with_conn(|c| embedding::untagged(c, 500)) else { break };
            if batch.is_empty() || cancel.is_cancelled() {
                break;
            }
            for (id, vec) in batch {
                let _ = lib.db.with_conn(|c| embedding::set_scene_tags(c, id, &embedding::scene_tags(&vec, &protos)));
            }
        }
        crate::faces::schedule(&a);
        crate::pets::schedule(&a);
        crate::ocr::schedule(&a);
        if total > 0 {
            let secs = started.elapsed().as_secs_f32().max(0.001);
            tracing::info!(embedded = total, per_second = total as f32 / secs, "embeddings backfilled");
            crate::maintenance::schedule_memories(&a);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_come_out_row_major_rgb() {
        let mut img = image::RgbImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        img.put_pixel(1, 0, image::Rgb([0, 0, 255]));
        let (rgb, w, h) = rgb_of(&image::DynamicImage::ImageRgb8(img));
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgb, vec![255, 0, 0, 0, 0, 255]);
    }
}
