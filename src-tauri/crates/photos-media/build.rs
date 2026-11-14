//! Builds `photos-raw` on its own (`--target-dir target/raw-lib`, profile `raw-lib`), so that a
//! `cargo test -p photos-media` or an example finds the library too. `src-tauri/build.rs` does
//! the same and copies it for the bundle; whichever runs second finds nothing to do.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.parent().and_then(Path::parent).unwrap();
    println!("cargo:rerun-if-changed=../photos-raw/src");
    println!("cargo:rerun-if-changed=../photos-raw/Cargo.toml");
    println!("cargo:rerun-if-changed=build.rs");
    let target_dir = workspace.join("target").join("raw-lib");
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.current_dir(workspace).args(["build", "-p", "photos-raw", "--profile", "raw-lib", "--target-dir"]).arg(&target_dir);
    // Not clippy's driver or an outer build's flags: this is a build of its own.
    for var in ["RUSTC_WORKSPACE_WRAPPER", "CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS"] {
        cargo.env_remove(var);
    }
    let (target, host) = (std::env::var("TARGET").unwrap(), std::env::var("HOST").unwrap());
    let mut out = target_dir;
    if target != host {
        cargo.args(["--target", &target]);
        out = out.join(&target);
    }
    let status = cargo.status().expect("running cargo for photos-raw");
    assert!(status.success(), "building photos-raw failed");
    let file = if target.contains("windows") {
        "photos_raw.dll"
    } else if target.contains("apple") {
        "libphotos_raw.dylib"
    } else {
        "libphotos_raw.so"
    };
    println!("cargo:rustc-env=PHOTOS_RAW_BUILT={}", out.join("raw-lib").join(file).display());
}
