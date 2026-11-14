//! Round 5A: a Develop preview while background work fills the global rayon pool, as an
//! import or a face scan does — with and without the render pool.
//!
//! ```sh
//! cargo test --release -p lumen --test render_under_load -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use photos_core::edits::{Develop, Recipe};

fn photo() -> image::DynamicImage {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(2048, 1365, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])))
}

fn recipe() -> Recipe {
    let mut d = Develop::default();
    d.basic.exposure = 0.4;
    d.basic.clarity = 30.0;
    d.basic.texture = 20.0;
    d.basic.dehaze = 15.0;
    Recipe { develop: Some(d), ..Recipe::identity() }
}

/// Median of `n` renders.
fn time(n: usize, render: impl Fn()) -> Duration {
    let mut t: Vec<Duration> = (0..n)
        .map(|_| {
            let s = Instant::now();
            render();
            s.elapsed()
        })
        .collect();
    t.sort();
    t[n / 2]
}

#[test]
#[ignore = "a timing, not a check"]
fn a_preview_under_background_load() {
    // As the app does at launch.
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let _ = rayon::ThreadPoolBuilder::new().num_threads(cores.saturating_sub(1).max(1)).start_handler(|_| photos_jobs::background_priority()).build_global();
    photos_jobs::interactive(|| ());
    let (img, r) = (photo(), recipe());
    let render = || drop(photos_media::develop::render(&img, &r, 1));

    let idle = time(7, render);

    println!("idle {idle:?}");
    // Background work as the app makes it: each job-lane worker (cores − 1 of them, below
    // normal) runs pixel loops with `par_iter`, so the global pool always has work queued.
    let busy = Arc::new(AtomicBool::new(true));
    let workers: Vec<_> = (0..cores.saturating_sub(1).max(1))
        .map(|_| {
            let busy = busy.clone();
            std::thread::spawn(move || {
                photos_jobs::background_priority();
                while busy.load(Ordering::Relaxed) {
                    use rayon::prelude::*;
                    let s: u64 = (0..256u64)
                        .into_par_iter()
                        .map(|k| {
                            let mut x = k;
                            for i in 0..200_000u64 {
                                x = x.wrapping_mul(31).wrapping_add(i);
                            }
                            x
                        })
                        .sum();
                    std::hint::black_box(s);
                }
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(200));
    let shared = time(5, render);
    println!("under load, global pool {shared:?}");
    let pooled = time(7, || photos_jobs::interactive(render));
    busy.store(false, Ordering::Relaxed);
    for w in workers {
        let _ = w.join();
    }

    println!("2048 px preview, median: idle {idle:?}; under load on the global pool {shared:?}; under load on the render pool {pooled:?}");
}
