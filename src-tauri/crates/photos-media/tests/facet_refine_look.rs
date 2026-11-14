//! A look at refined AI masks on real photos, without the model: the public photos and rasters of
//! `photos-ml`'s Facet fixtures, drawn as the raster's edge (left) and the refined one (right).
//!
//! `LUMEN_LOOK_DIR=<temp folder> cargo test --release -p photos-media --test facet_refine_look -- --ignored --nocapture`

use std::path::PathBuf;

use image::{Rgb, RgbImage};
use photos_core::ai_masks::{AiMaskKind, AiMaskRaster, AiRasters};
use photos_core::edits::{Mask, MaskShape};
use photos_media::develop::masks::{rasterize_with, FrameMap, Guide};
use photos_media::develop::Linear;
use serde::Deserialize;

#[derive(Deserialize)]
struct Photo {
    file: String,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    kind: AiMaskKind,
    instance: u32,
    width: u32,
    height: u32,
    /// Run lengths of weights ≥ 128, starting with a run of 0s.
    rle: Vec<usize>,
}

fn fixture(name: &str) -> &'static [u8] {
    match name {
        "crew.jpg" => include_bytes!("../../photos-ml/tests/fixtures/facet/crew.jpg"),
        "valley.jpg" => include_bytes!("../../photos-ml/tests/fixtures/facet/valley.jpg"),
        "car.jpg" => include_bytes!("../../photos-ml/tests/fixtures/facet/car.jpg"),
        other => panic!("no fixture {other}"),
    }
}

fn tinted(img: &RgbImage, weights: &[f32]) -> RgbImage {
    let mut out = img.clone();
    for (p, w) in out.pixels_mut().zip(weights) {
        *p = Rgb(std::array::from_fn(|c| (p[c] as f32 * (1.0 - 0.55 * w) + [255.0, 40.0, 40.0][c] * 0.55 * w) as u8));
    }
    out
}

#[test]
#[ignore = "writes pictures to LUMEN_LOOK_DIR"]
fn draws_raster_and_refined_edges_side_by_side() {
    let dir = PathBuf::from(std::env::var("LUMEN_LOOK_DIR").expect("LUMEN_LOOK_DIR"));
    let golden: Vec<Photo> = serde_json::from_str(include_str!("../../photos-ml/tests/fixtures/facet/golden.json")).unwrap();
    for photo in &golden {
        let img = image::load_from_memory(fixture(&photo.file)).unwrap().to_rgb8();
        let (w, h) = img.dimensions();
        let frame = Linear::from_rgb8_downscaled(&img, w.max(h));
        let map = FrameMap::identity(w, h);
        let guide = Guide::new(&frame, map.long_edge(), (0, 0));
        for row in &photo.rows {
            let mut weights = Vec::with_capacity((row.width * row.height) as usize);
            for (i, &n) in row.rle.iter().enumerate() {
                weights.extend(std::iter::repeat_n(if i % 2 == 1 { 255u8 } else { 0 }, n));
            }
            let raster = AiMaskRaster { kind: row.kind, instance: row.instance, width: row.width, height: row.height, weights };
            let store = AiRasters::from([("aa".to_string(), raster)]);
            let shape = MaskShape::Ai { add: true, target: row.kind, instance: row.instance, raster: "aa".into() };
            let mask = Mask { id: 1, name: "m".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust: Default::default(), visible: true };
            let coarse = rasterize_with(&mask, &map, w, h, Some(&store), None);
            let fine = rasterize_with(&mask, &map, w, h, Some(&store), Some(&guide));
            let mut sheet = RgbImage::new(w * 2 + 8, h);
            image::imageops::replace(&mut sheet, &tinted(&img, &coarse), 0, 0);
            image::imageops::replace(&mut sheet, &tinted(&img, &fine), w as i64 + 8, 0);
            let name = format!("{}-{}-{}.jpg", photo.file.trim_end_matches(".jpg"), row.kind.as_str(), row.instance);
            sheet.save(dir.join(&name)).unwrap();
            println!("{name}: {w}x{h}, raster {}x{}", row.width, row.height);
        }
    }
}
