# Round 11 — Typed dates, date order, Lightroom presets and catalogs, Retouch (2026-10-06)

Julian's list of 2026-10-06, after Round 10 and the *apple-design* fixes merged into `main`. Two sessions: **11A** (`plans/2026-10-06-round-11a-dates-lightroom.md`) and **11B** (`plans/2026-10-06-round-11b-retouch.md`).

## What Julian asked for

1. Make *date taken* and *date modified* dynamic so he can type a specific date (`date taken: 4.6.2026`).
2. A setting for the date format, detected from the region / system and changeable, so Europeans and Americans each type and read their own format. It applies to search **and every date shown**.
3. A **Retouch** button beside *Auto* in Develop that sets every panel (crop, Basic, effects, masks, tone curve, colour grading, detail…) with a custom model trained on real photographers' edits, plus training scripts for this and Lumen's later custom models, so it can be trained well.
4. Lightroom presets imported and converted into Lumen presets.
5. *Import a Lightroom library* on Import, shown when one is found.

## Julian's answers (2026-10-06)

- Retouch learns from **RAW + Lightroom edits** (original + a photographer's settings), **finished photos** (a look, with no before), and **his own edits in Lumen** (an on-device personal layer).
- Masks: Retouch uses the gradients that exist today (linear, radial). Subject/sky masks wait for **8f**.
- A Lightroom catalog's develop settings go **into `edits`**, so photos look as they did in Lightroom (originals untouched).
- The date order applies to **search and every date shown**.

## Assumptions (stated in the design, not objected to)

- "Dynamic" means any date typed after a field name becomes a chip at once, at any precision (day, month, year), with `before` / `after` / `between … and …` / `–` ranges and `field:` syntax.
- "Lightroom library" means **Lightroom Classic** (`.lrcat`, SQLite). Lightroom (cloud) has no local catalog.
- **The shipped Retouch weights train only on data whose licence allows commercial use** (`commercial-freemium-release`). Julian's own catalog is used to evaluate, and on his machine for his personal layer. It never trains shipped weights.

## 11A — dates, date order, Lightroom

### Date order
- `photos-core::settings`: `SearchSettings { date_order: DateOrderSetting }` with `Auto | Dmy | Mdy | Ymd`, defaulting to `Auto`. `Auto` resolves from the OS locale (`sys-locale` crate in the binary) to `DateOrder::{Dmy, Mdy, Ymd}`. `en-US`, `en-PH`, `fil`, `en-CA` (and `en-BZ`, `en-FM`, `en-MH`, `en-PW`) read MDY. `zh`, `ja`, `ko`, `hu`, `lt`, `mn`, `en-ZA`, `sv-SE`… read YMD. Everyone else reads DMY.
- `query_parse::Clock` carries the order. `a.b.yyyy` reads in that order only. A reading impossible in the order (e.g. 25/8/2023 under MDY) falls back to the other one, because typing it can only mean that. `yyyy-mm-dd` is always ISO.
- The frontend's `src/utils/date.ts` formats with the resolved order (numeric short forms) and keeps the locale's month names for long forms. Every caller of `Intl.DateTimeFormat` outside `utils/date.ts` moves into it.
- Settings → **Search & dates**: *Date order* with *Automatic (Day-month-year)* naming what it resolved to, and an example under the control (`4.6.2026 = 4 June 2026`).

### Typed dates per field
- `tag_search::suggest` makes a live chip for any phrase `field [:|=]? <date expression>`, for every `DateField`. `<date expression>` is anything `read_date` reads, or `before X`, `after X`, `since X`, `until X`, `between X and Y`, `X – Y` / `X - Y` / `X to Y`. Labels name the field and the span: *Taken: 4 June 2026*, *File modified: before March 2024*, *Taken: 1 – 4 June 2026*.
- Spans are inclusive and in the field's own clock (wall-clock for *Taken* / *Digitized*, UTC for file times), as Round 10 made them.

### Lightroom → Lumen converter (`photos-core::lightroom`)
- **Input:** a map of Camera Raw settings (`crs:` names) from three readers. **XMP** (preset files, sidecars): `crs:` attributes and elements, `rdf:Seq` lists for curves, `crs:MaskGroupBasedCorrections`. **`.lrtemplate`**: the Lua table under `value.settings`. **Catalog develop text**: the Lua table in `Adobe_imageDevelopSettings.text`. One small Lua-table reader serves the last two. It is not a Lua interpreter: tables, strings, numbers, booleans.
- **Output:** `Converted { develop: Develop, geometry: Option<Geometry>, panels: Vec<Panel>, skipped: Vec<String> }`. `Geometry` is `{ crop: Option<Crop>, straighten: f32 }`. `panels` lists those the settings touched, so a preset sets only them. `skipped` lists the settings with no Lumen equivalent (profiles, lens and transform corrections, brush and AI masks, *Auto* flags, calibration…).
- **Mapping (process version 2012+; PV2010 and older are approximated and flagged in `skipped`):** `Exposure2012` → exposure (EV, same scale). `Contrast2012`, `Highlights2012`, `Shadows2012`, `Whites2012`, `Blacks2012`, `Texture`, `Clarity2012`, `Dehaze`, `Vibrance`, `Saturation` → /100. `IncrementalTemperature` / `IncrementalTint` → temperature / tint (same −100..100). Kelvin `Temperature`/`Tint` on a RAW: `temperature = clamp(100 · log2(K / as_shot_K) / 1.2)`, with `as_shot_K` from the photo when known, else 5500 K, and flagged as approximated. `ToneCurvePV2012[Red|Green|Blue]` points /255 → `curve`. `Parametric*` sliders and splits → five curve points added to master. `Hue/Saturation/LuminanceAdjustment<Band>` → `hsl[band]` /100. Colour grading (`ColorGrade*`, `SplitToning*`) → wheels (hue degrees, sat /100, lum /100), `ColorGradeBlending` /100, `SplitToningBalance` /100. `Sharpness` /100, `SharpenRadius`, `SharpenDetail` /100, `SharpenEdgeMasking` /100. `LuminanceSmoothing` > 0 → `denoise.strength` /100. `PostCropVignetteAmount` /100, `…Midpoint` /100, `…Feather` /100. `GrainAmount` /100, `GrainSize` /100. `CropLeft/Top/Right/Bottom` → `Crop{x,y,w,h}` and `CropAngle` → straighten, when `HasCrop`. `Mask/Gradient` (`ZeroX/Y`, `FullX/Y`) → `MaskShape::Linear`. `Mask/CircularGradient` (`Top/Left/Bottom/Right/Angle/Feather/Flipped`) → `MaskShape::Radial`. `Local*2012` / `LocalTemperature`… → `LocalAdjust` (local sliders −1..1 already, except `LocalExposure2012` in EV).
- Every converted `Develop` is `.normalized()`. Golden file: `src-tauri/crates/photos-core/tests/lightroom-golden.json` (inputs → outputs), regenerated by the Rust test, never edited by hand. 11B's training scripts read it too.
- A dev CLI, `cargo run -p photos-core --example lr2lumen -- <file.xmp|.lrtemplate>` prints the `Converted` JSON. 11B's dataset builder calls it, so Python and Rust share one mapping.

### Preset import
- Develop → presets list → **Import Lightroom presets…** opens a file/folder picker (`.xmp`, `.lrtemplate`, recursive in a folder). When Camera Raw's settings folder exists (`%APPDATA%/Adobe/CameraRaw/Settings`, `~/Library/Application Support/Adobe/CameraRaw/Settings`), a second item *Import from Lightroom* reads it directly. A preset's name is its `crs:Name` / `title`, else the file stem. Its group (`crs:Group`) prefixes the name (`Portrait / Soft`). Same-name presets are skipped, not duplicated. Presets never carry masks (existing rule): a preset's gradients go into `skipped`.
- Toast: *Imported 34 Lightroom presets · 3 settings not converted* with *Details* listing them per preset.

### Lightroom Classic catalog import (`photos-import::lightroom`)
- **Detection** (`find_lightroom_catalogs`, cached 60 s like Apple's): `*.lrcat` directly in `Pictures/Lightroom`, home, each drive's top and `Pictures`, plus the catalogs Lightroom's preferences list as recent (Windows `%APPDATA%/Adobe/Lightroom/Preferences/Lightroom Classic CC 7 Preferences.agprefs`, macOS `~/Library/Preferences/com.adobe.LightroomClassicCC7.plist`, best effort). Import shows *Lightroom Classic catalog…* and a card per catalog only when one is found.
- **Reading:** a temp copy of the `.lrcat` (Lightroom holds it locked while open), opened read-only. Tables: `AgLibraryRootFolder.absolutePath` + `AgLibraryFolder.pathFromRoot` + `AgLibraryFile.baseName.extension` → the original's path. `Adobe_images` gives rating, pick, colour label, capture time and virtual copies (`masterImage` not null, skipped and counted). `Adobe_imageDevelopSettings.text` holds the develop settings. Also `AgLibraryKeyword(Image)`, `AgLibraryCollection(Image)` (only `com.adobe.ag.library.collection` collections; smart collections listed as skipped), and `AgLibraryIPTC` caption, plus the title when present. Columns are verified against a real catalog before the fixture is written.
- **Importing:** `LightroomCatalogSource` implements `ImportSource`. It enumerates originals that exist, lists missing or offline ones in the import summary, and opens files read-only. `ItemExtras` gains `rating: Option<i64>`, `flag: Option<i64>`, `color_label: Option<i64>`, `recipe: Option<Recipe>`. `extras::apply` writes them the same additive way: it never lowers a rating or overwrites an existing edit. The recipe is the converted develop plus geometry, saved through the normal `edits` path, so thumbnails re-render. A JPEG + RAW pair in Lightroom (sidecar `JPG`) imports both, as a folder import would.
- If the catalog needs a migration, it is **0031**. 11B takes none.

## 11B — Retouch

### What it does
- *Retouch* (button beside *Auto*, palette *Retouch photo*, key `Mod+Shift+U`) writes one recipe to the Develop draft. It covers crop + straighten, Basic, curve, HSL, grading, detail (sharpen; denoise only if already on), effects, and at most one linear and one radial gradient. One undo step. The toast says what it did: *Retouch — straightened 1.4°, cropped, sky darkened*.
- Without the model (smart features off, still downloading) it falls back to *Auto* (8d) and says so. It never fails silently.

### The model: slider values, not pixels
- **Inputs:** the photo at its preview base, 256 px long edge, padded to 256×256, linear, log-encoded (`log2(x + 1/64)` scaled); a 64-bin luminance histogram plus per-channel means and percentiles (the 8d measurement); the stored CLIP image embedding (512, zeros when missing, with a flag).
- **Outputs:** one vector in `[0,1]` per slider, laid out by `photos-core::retouch::PARAMS` (name, min, max, neutral) and exported to `src-tauri/crates/photos-core/tests/retouch-params.json` by a Rust test. Python reads that file and never hard-codes ranges. Mask heads add a presence probability; masks below 0.5 are dropped. Crop is `{x,y,w,h}` with an aspect-preserving head (the crop's ratio from {original, 1:1, 4:5, 3:2, 16:9} plus a free box).
- **Network:** a small CNN trained from scratch (no third-party image weights to license) on the 256 px input, concatenated with the histogram and CLIP features, then an MLP to the heads. ~2–5 M parameters, ONNX fp32 < 20 MB, < 40 ms on CPU.
- **Loss:** a weighted per-slider L1 in normalised units, plus **ΔE (CIEDE2000) between renders** with a differentiable PyTorch copy of Lumen's global stages (Basic, curve, HSL, grading, vignette, linear/radial masks), so two slider sets that look alike are not punished. The PyTorch render matches Rust within 1/255 on `develop-render-golden.json` (Rust-generated test images and recipes). Spatial stages (clarity, texture, dehaze, sharpen, grain) train on the parameter loss only. Crop uses IoU plus a parameter loss; straighten uses an angle loss.

### Training data
Every source is registered in `scripts/ml/lumen_train/datasets/registry.toml` with its licence, URL and `commercial = true|false`. `train.py --ship` refuses a source that is not commercial. Each is vetted and recorded in `docs/THIRD-PARTY-MODELS.md` **before** it is downloaded.
1. **RAW + Lightroom pairs:** a folder of originals with `.xmp` sidecars (or a `.lrcat`). The pair builder converts each through `lr2lumen` (11A), prepares the input with the app's own decoder (`cargo run -p lumen --example retouch_prep`), and stores `input.f16`, `features.json` and `target.json`. Candidates: pixls.us *Play Raw* (CC BY-SA 4.0, several photographers' edits per RAW; vet whether share-alike reaches trained weights) and Julian's own catalog (**evaluation and personal layer only**).
2. **Finished photos:** commercially licensed edited photos. Because global adjustments are near-invertible, each final becomes a **synthetic pair**: random degradation recipes (exposure ±2 EV, white balance ±40, contrast/saturation ±0.4, curve flattening, tilt ±6°, and a random looser crop by padding with a mirror) are applied with Lumen's renderer, and the model learns to undo them. This teaches exposure, white balance, horizon and look at scale. Candidate sources are vetted per licence for AI training (Unsplash's and Pexels' terms are checked first; only those that allow it are used).
3. **Julian's own edits (on device):** whenever a Develop draft is exported, Lumen stores `(features, final recipe as the PARAMS vector, model output)` in `config_dir/JL-Studios/Lumen/retouch/<library-id>.json` (capped at 2,000 pairs, newest kept). A ridge-regression **personal layer** in Rust (`photos-core::retouch::personal`) learns `residual = final − model` from the features and is added to the model's output once ≥ 30 pairs exist. Settings → Smart features → *Retouch learns from your edits* (on) and *Reset*. Nothing leaves the machine. No migration.

### Training kit (`scripts/ml/lumen_train/`, shared by Lumen's custom models)
- `common/`: config (TOML), seeding, device pick (CUDA on Julian's RTX 3080 Ti, else CPU), checkpoint/resume, CSV + TensorBoard logs, ONNX export with an onnxruntime parity check (max abs diff < 1e-4), SHA-256 and a `models-v1` manifest entry, licence gate.
- `retouch/`: `build_pairs.py`, `synth.py`, `render.py` (the differentiable copy), `model.py`, `train.py`, `eval.py`, `export.py`.
- `requirements-train.txt` with CUDA torch, separate from the CPU export environment (`.venv-train`).
- `docs/ML-TRAINING.md`: set up, add a dataset, train, evaluate, export, and add the next model.
- **Evaluation** (`eval.py`): a held-out split by photographer and by scene. It reports mean ΔE to the photographer's render, per-panel slider error, crop IoU and angle error, each beside *Auto*'s. It also writes an HTML contact sheet (original | Auto | Retouch | photographer) where Julian marks *better / same / worse*, in the style of the radar sheets. **Ship bar:** lower ΔE than Auto on the held-out set, and Retouch marked better or same on ≥ 80 % of Julian's sheet.
- Upload to `models-v1` only with Julian's OK, as for C.

### In the app
- `photos-ml::retouch`: an ONNX session (`retouch-v1.onnx` in the manifest, optional).
- `photos-core::retouch`: `PARAMS`, `decode(vec) -> RetouchRecipe { develop, crop, straighten, summary }`, `encode(recipe) -> vec` (for the personal layer), and `personal::{record, fit, apply}`.
- `src-tauri/src/develop_retouch.rs` (beside `develop_auto.rs`): builds the inputs from the preview base, runs the model, applies the personal layer, and decodes. Command `develop_retouch(asset_id) -> RetouchResult { develop, crop, straighten, used_model, personal, summary }`. The Develop session applies it as one history step.

## Out of scope
- AI subject/sky masks (8f) and brush masks in Retouch.
- Lightroom (cloud) libraries, Capture One, Lightroom's lens/profile corrections.
- Training on Julian's library for shipped weights.
