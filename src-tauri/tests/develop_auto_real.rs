//! Phase 8d: *Auto* on real photos. Reads a folder of JPEG **copies** (never the library's own
//! folder), runs Auto as the app does at 256 px, and prints per photo the scene, the prior, the
//! result, the aesthetic before and after and the time.
//!
//! ```sh
//! LUMEN_MODELS_DIR=<models> LUMEN_AUTO_FOLDER=<folder of copies> cargo test --release -p lumen --test develop_auto_real -- --ignored --nocapture
//! ```

use std::path::Path;
use std::time::Instant;

use image::RgbImage;
use lumen_lib::develop_auto::auto_with;
use photos_core::develop_auto::{measure, prior, scene_from};
use photos_core::edits::{Basic, Develop};
use photos_media::develop::{develop_stages, Ctx, Linear};
use photos_ml::clip::{ImageEncoder, Rgb};
use photos_ml::manifest::manifest;

fn render(small: &Linear, basic: Basic) -> RgbImage {
    let mut img = small.clone();
    develop_stages(&mut img, &Develop { basic, auto: true, ..Default::default() }.normalized(), &Ctx::new(1.0, 1));
    img.to_rgb8()
}

fn aesthetic(enc: &mut ImageEncoder, img: &RgbImage) -> f32 {
    enc.embed(img.as_raw(), img.width(), img.height()).unwrap().1.unwrap_or(0.5)
}

#[test]
#[ignore = "needs the models and a folder of photo copies"]
fn auto_improves_real_photos_quickly() {
    let models = std::env::var("LUMEN_MODELS_DIR").expect("LUMEN_MODELS_DIR");
    let folder = std::env::var("LUMEN_AUTO_FOLDER").expect("LUMEN_AUTO_FOLDER");
    let entry = manifest().by_role("image_embedding").unwrap();
    let mut enc = ImageEncoder::open(&Path::new(&models).join(&entry.file), entry).unwrap();
    let protos = photos_ml::prototypes();
    let (mut worse, mut slowest, mut n) = (0, 0f64, 0);
    for f in std::fs::read_dir(&folder).unwrap() {
        let path = f.unwrap().path();
        let Ok(img) = image::open(&path) else { continue };
        n += 1;
        let t = Instant::now();
        let small = Linear::from_dynamic(&img).resize_long_edge(256);
        let (v, _) = enc.embed(small.to_rgb8().as_raw(), small.width, small.height).unwrap();
        let scene = scene_from(&v, &protos);
        let judge = |imgs: &[RgbImage]| -> Result<Vec<f32>, String> {
            let batch: Vec<Rgb<'_>> = imgs.iter().map(|i| Rgb { data: i.as_raw(), width: i.width(), height: i.height() }).collect();
            Ok(enc.embed_batch(&batch).map_err(|e| e.to_string())?.into_iter().map(|(_, a)| a.unwrap_or(0.5)).collect())
        };
        let (best, used) = auto_with(&small, &Develop::default(), scene, &Ctx::new(1.0, 1), 7, Some(judge)).unwrap();
        let took = t.elapsed().as_secs_f64();
        assert!(used);
        let p = prior(&measure(&small.data), &scene);
        let (a0, a1) = (aesthetic(&mut enc, &render(&small, p)), aesthetic(&mut enc, &render(&small, best)));
        let name = path.file_name().unwrap().to_string_lossy();
        println!(
            "{name}: scene {:?} {:?} top {:?}; prior exp {:+.2} con {:+.2} temp {:+.0} tint {:+.0}; auto exp {:+.2} con {:+.2} hi {:+.2} sh {:+.2} temp {:+.0} tint {:+.0} vib {:+.2}; aesthetic {a0:.3} → {a1:.3}; {took:.2} s",
            scene.strongest(),
            scene,
            photos_core::embedding::softmax(&v, &protos, "scene:", photos_core::develop_auto::SCENE_TEMPERATURE).into_iter().take(2).collect::<Vec<_>>(),
            p.exposure,
            p.contrast,
            p.temperature,
            p.tint,
            best.exposure,
            best.contrast,
            best.highlights,
            best.shadows,
            best.temperature,
            best.tint,
            best.vibrance
        );
        // Below 0.02 the head's scores are noise (a screenshot scoring 0.014 → 0.000 won on
        // clipping, which the search weighs too).
        if a1 + 0.02 < a0 {
            worse += 1;
        }
        slowest = slowest.max(took);
    }
    println!("{n} photos; aesthetic worse than the prior on {worse}; slowest {slowest:.2} s");
    assert!(n > 0);
    assert_eq!(worse, 0, "Auto never scores below its own prior");
    assert!(slowest < 4.0);
}
