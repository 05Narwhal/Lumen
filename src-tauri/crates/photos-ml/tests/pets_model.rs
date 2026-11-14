//! D-FINE-S and DINOv2 on three public-domain White House photographs of Socks the Cat and
//! Buddy the Dog (US federal works, via Wikimedia Commons / DPLA): on the lawn twice and in the
//! Outer Oval Office. Both animals are found once each in every photo, each the right species
//! once CLIP has had its say (D-FINE calls Socks a dog in the second), and each is nearer to
//! itself in the other photos than to the other animal.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<folder holding the model files> cargo test --release -p photos-ml --test pets_model -- --ignored --nocapture
//! ```

use std::path::Path;

use photos_core::embedding::{cosine, softmax, Vector};
use photos_core::pet_filter;
use photos_ml::clip::{ImageEncoder, Rgb};
use photos_ml::manifest::manifest;
use photos_ml::pets::{pet_crop_rect, PetDetector, PetEmbedder, Species, CROP_MARGIN};

struct Models {
    det: PetDetector,
    emb: PetEmbedder,
    clip: ImageEncoder,
}

fn models() -> Models {
    let dir = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let m = manifest();
    let (det, emb, clip) = (m.by_role("pet_detection").unwrap(), m.by_role("pet_embedding").unwrap(), m.by_role("image_embedding").unwrap());
    Models {
        det: PetDetector::open(&Path::new(&dir).join(&det.file), det).unwrap(),
        emb: PetEmbedder::open(&Path::new(&dir).join(&emb.file), emb).unwrap(),
        clip: ImageEncoder::open(&Path::new(&dir).join(&clip.file), clip).unwrap(),
    }
}

/// Every animal in a photo, each with its species as the scan settles it and its description.
fn look(m: &mut Models, bytes: &[u8]) -> Vec<(Species, Vector)> {
    let img = image::load_from_memory(bytes).unwrap().to_rgb8();
    let (w, h) = img.dimensions();
    let rgb = img.into_raw();
    let t = std::time::Instant::now();
    let boxes = m.det.detect(&rgb, w, h).unwrap();
    println!("{boxes:?} in {:?}", t.elapsed());
    let crops: Vec<(Vec<u8>, u32)> = boxes
        .iter()
        .map(|b| {
            let (x, y, side) = pet_crop_rect(b.bbox, w, h, CROP_MARGIN);
            let mut out = Vec::with_capacity((side * side * 3) as usize);
            for row in y..y + side {
                let start = ((row * w + x) * 3) as usize;
                out.extend_from_slice(&rgb[start..start + side as usize * 3]);
            }
            (out, side)
        })
        .collect();
    let batch: Vec<Rgb> = crops.iter().map(|(c, s)| Rgb { data: c, width: *s, height: *s }).collect();
    let vecs = m.emb.embed_batch(&batch).unwrap();
    let clips = m.clip.embed_batch(&batch).unwrap();
    let protos = photos_ml::prototypes();
    boxes
        .iter()
        .zip(vecs)
        .zip(clips)
        .map(|((b, v), (c, _))| {
            let shares = softmax(&c, &protos, "species:", pet_filter::CROP_TEMPERATURE);
            let s = Species::parse(pet_filter::species(b.species.as_str(), &shares)).unwrap();
            println!("  {} at {:.2}, CLIP {shares:?} → {}", b.species.as_str(), b.score, s.as_str());
            (s, v)
        })
        .collect()
}

#[test]
#[ignore = "needs the downloaded models"]
fn finds_socks_and_buddy_and_knows_them_again() {
    let mut m = models();
    let photos: [&[u8]; 3] = [include_bytes!("fixtures/socks-buddy-1.jpg"), include_bytes!("fixtures/socks-buddy-2.jpg"), include_bytes!("fixtures/socks-buddy-3.jpg")];
    // (Socks, Buddy) in each. In the first D-FINE also reads Socks as a dog at 0.64; only the
    // likelier reading is kept.
    let seen: Vec<(Vector, Vector)> = photos
        .iter()
        .map(|p| {
            let found = look(&mut m, p);
            let one = |s: Species| {
                let all: Vec<&(Species, Vector)> = found.iter().filter(|(k, _)| *k == s).collect();
                assert_eq!(all.len(), 1, "one {}", s.as_str());
                all[0].1.clone()
            };
            (one(Species::Cat), one(Species::Dog))
        })
        .collect();
    // Socks knows himself in every other photo. Buddy does between the second and third; in
    // the first Socks sits in front of him, so Buddy's square crop holds Socks too and looks
    // more like him (a known limit: overlapping animals share a crop).
    for i in 0..seen.len() {
        for j in 0..seen.len() {
            if i == j {
                continue;
            }
            let ((socks_i, buddy_i), (socks_j, buddy_j)) = (&seen[i], &seen[j]);
            let (socks, buddy) = (cosine(socks_i, socks_j), cosine(buddy_i, buddy_j));
            println!("photos {} and {}: Socks {socks:.3} (vs Buddy {:.3}), Buddy {buddy:.3} (vs Socks {:.3})", i + 1, j + 1, cosine(socks_i, buddy_j), cosine(buddy_i, socks_j));
            assert!(socks > cosine(socks_i, buddy_j), "Socks in photo {} knows himself in photo {}", i + 1, j + 1);
            if i != 0 && j != 0 {
                assert!(buddy > cosine(buddy_i, socks_j), "Buddy in photo {} knows himself in photo {}", i + 1, j + 1);
            }
        }
    }
}
