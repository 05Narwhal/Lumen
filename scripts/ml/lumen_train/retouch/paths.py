"""Where the files Rust writes for the training scripts live in the repository, and the Rust
examples the scripts run."""

import functools
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
SRC_TAURI = REPO / "src-tauri"
# Written by `cargo test -p photos-core --lib retouch` (the slider layout and sample decodes).
PARAMS_JSON = SRC_TAURI / "crates" / "photos-core" / "tests" / "retouch-params.json"
# Written by `cargo test -p photos-media --test develop_render_golden`.
RENDER_GOLDEN = SRC_TAURI / "crates" / "photos-media" / "tests" / "develop-render-golden.json"

# Which crate holds each example.
_CRATES = {"lr2lumen": "photos-core", "retouch_prep": "photos-media", "render_recipe": "photos-media", "retouch_auto": "photos-media"}


@functools.cache
def example(name: str) -> Path:
    """A release build of a Rust example, built (or brought up to date) once per run, so the
    scripts never use a binary older than the code that prepares their data."""
    subprocess.run(["cargo", "build", "--release", "-p", _CRATES[name], "--example", name], cwd=SRC_TAURI, check=True)
    return SRC_TAURI / "target" / "release" / "examples" / (name + (".exe" if sys.platform == "win32" else ""))
