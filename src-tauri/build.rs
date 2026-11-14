use std::path::{Path, PathBuf};
use std::process::Command;

/// Builds `photos-raw` on its own and copies it to `binaries/raw/`, before Tauri checks that
/// `bundle.resources` exists. The dependency's build script (`photos-media`) does the build too,
/// because `cargo test -p photos-media` never runs this one: whichever comes second has nothing to do.
fn raw_library() {
    let workspace = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed=crates/photos-raw/src");
    println!("cargo:rerun-if-changed=crates/photos-raw/Cargo.toml");
    let target_dir = workspace.join("target").join("raw-lib");
    let mut cargo = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cargo.current_dir(&workspace).args(["build", "-p", "photos-raw", "--profile", "raw-lib", "--target-dir"]).arg(&target_dir);
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
    let dest: &Path = &workspace.join("binaries").join("raw");
    std::fs::create_dir_all(dest).expect("binaries/raw");
    std::fs::copy(out.join("raw-lib").join(file), dest.join(file)).expect("copying photos-raw");
}

fn main() {
    raw_library();
    embed_discord_client_id();
    embed_plugin_pubkey();
    tauri_build::build()
}

/// Bake the Discord Application ID (Rich Presence) into the binary.
///
/// A public identifier, not a credential. It is injected rather than committed because it is
/// per-deployment: a fork must not publish a status as Lumen. Resolved from
/// `LUMEN_DISCORD_CLIENT_ID` or a git-ignored id file.
///
/// Absent is a supported state: `discord_presence::client_id()` returns `None`, Settings hides
/// the Discord status section and the worker never starts. Never an error.
fn embed_discord_client_id() {
    println!("cargo:rerun-if-env-changed=LUMEN_DISCORD_CLIENT_ID");

    let id_files = ["discord_client_id.txt", ".discord_client_id", "../discord_client_id.txt", "../.discord_client_id"];
    for f in id_files {
        println!("cargo:rerun-if-changed={f}");
    }

    let resolved = std::env::var("LUMEN_DISCORD_CLIENT_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            id_files.iter().find_map(|f| {
                std::fs::read_to_string(f).ok().map(|c| c.trim().to_string()).filter(|c| !c.is_empty())
            })
        });

    match resolved {
        Some(id) => println!("cargo:rustc-env=LUMEN_DISCORD_CLIENT_ID={id}"),
        None => println!(
            "cargo:warning=LUMEN_DISCORD_CLIENT_ID not set and no id file found - the Discord status is UNAVAILABLE in this build. Create src-tauri/discord_client_id.txt or set $env:LUMEN_DISCORD_CLIENT_ID."
        ),
    }
}

/// Bake the JL Studios plugin public key into the binary (Round 23, spec §8), from
/// `LUMEN_PLUGIN_PUBKEY` or the git-ignored `src-tauri/plugin-pubkey.txt`: a minisign public key
/// line, the two-line `.pub` file, or the base64 text `tauri signer` writes.
///
/// Absent is a supported state: no package is *From JL Studios*. Never an error.
fn embed_plugin_pubkey() {
    println!("cargo:rerun-if-env-changed=LUMEN_PLUGIN_PUBKEY");
    println!("cargo:rerun-if-changed=plugin-pubkey.txt");
    let resolved = std::env::var("LUMEN_PLUGIN_PUBKEY")
        .ok()
        .or_else(|| std::fs::read_to_string("plugin-pubkey.txt").ok())
        // A `.pub` file's key is its last line (the first is an untrusted comment).
        .and_then(|v| v.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string));
    if let Some(key) = resolved {
        println!("cargo:rustc-env=LUMEN_PLUGIN_PUBKEY={key}");
    }
}
