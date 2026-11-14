//! Renders a recipe on a photo with the app's own pipeline, for Retouch's training targets and
//! evaluation sheets.
//!
//! `cargo run --release -p photos-media --example render_recipe -- <photo> <recipe.json> <out.png> [--edge N]`
//!
//! `recipe.json` is a `Recipe` (or a bare `Develop`). The photo is only read.

use std::path::PathBuf;

use photos_core::edits::{Develop, Recipe};
use photos_media::retouch_input::render_file;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let edge = args.iter().position(|a| a == "--edge").and_then(|i| args.get(i + 1)).and_then(|e| e.parse().ok()).unwrap_or(1024);
    let positional: Vec<&String> = args.iter().enumerate().filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--edge")).map(|(_, a)| a).collect();
    let [photo, recipe, out] = positional[..] else {
        eprintln!("usage: render_recipe <photo> <recipe.json> <out.png> [--edge N]");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(recipe).expect("read the recipe");
    let recipe: Recipe = serde_json::from_str(&text)
        .or_else(|_| serde_json::from_str::<Develop>(&text).map(|d| Recipe { develop: Some(d), ..Recipe::identity() }))
        .expect("a Recipe or a Develop");
    let img = render_file(&PathBuf::from(photo), &recipe, edge).expect("render");
    img.save(out).expect("write the image");
}
