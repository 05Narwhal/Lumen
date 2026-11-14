# Training Lumen's own models

Lumen ships two kinds of model: third-party ones it exports (`scripts/ml/export_*.py`, `.venv-ml`, CPU) and **its own**, trained here with the kit in `scripts/ml/lumen_train/` (`.venv-train`, CUDA). The first of its own is **Retouch** (Round 11B, spec `docs/superpowers/specs/2026-10-06-round-11-design.md`).

**Data rules, before anything else:**
- Every source goes in `scripts/ml/lumen_train/datasets/registry.toml` and `docs/THIRD-PARTY-MODELS.md` **before** it is downloaded: licence, URL, date checked, `commercial = true|false`. `--ship` training refuses a non-commercial source, and the checkpoint records the sources it saw.
- Julian's library is never a source. His own catalog or exports are used only from a folder he names, read-only, for evaluation and his on-device personal layer, never for shipped weights.
- Prepared data, checkpoints and runs live **outside the repository and the library**, e.g. `D:/lumen-train/`. `scripts/ml/lumen_train/runs/` is git-ignored for small local runs.

## Set up

```powershell
py -3.12 -m venv .venv-train                     # in the repository root (git-ignored)
.venv-train/Scripts/python -m pip install -r scripts/ml/requirements-train.txt
.venv-train/Scripts/python -c "import torch; print(torch.cuda.is_available())"
cd scripts/ml; ../../.venv-train/Scripts/python -m pytest lumen_train -q
```

The Rust side writes the files the scripts are checked against, so run its tests first after changing the layout or the pipeline: `cargo test -p photos-core --lib retouch` (`retouch-params.json`) and `cargo test -p photos-media --test develop_render_golden` (`develop-render-golden.json`).

## Retouch, end to end

1. **Prepare** photos exactly as the app does (decoding, orientation, RAW sensor data, 256 px, features):
   `cargo run --release -p photos-media --example retouch_prep -- <folder> D:/lumen-train/<source> --auto --jobs 8`
   Each photo becomes `<stem>-<hash>/{input.u16, meta.json}`. Name the folder after its registry source.
2. **Pairs** (optional, after 11A merges its `lr2lumen` converter): a folder of originals with Lightroom `.xmp` sidecars becomes targets with `python -m lumen_train.retouch.build_pairs <folder> D:/lumen-train/<source>`, where `<source>` is a registered source of kind `pairs`. The target is rendered by Lumen with only what training's render scores (`global_look`: no geometry, no texture/clarity/dehaze/sharpening/grain, only the two gradients the layout holds). Catalogs (`.lrcat`) are not read yet. A pairs source trains only on its photos that have a target.
3. **Train:** `python -m lumen_train.retouch.train --config lumen_train/retouch/config.toml data.root=D:/lumen-train run.dir=D:/lumen-train/runs/r1 [--ship]`. Finished photos become synthetic pairs on the fly (`synth.py`); pairs train on their targets. A run resumes from `last.pt` only with the sources and licence mode it started with: a run started without `--ship` never becomes shippable (start a new `run.dir`). The validation split is by photo name, so a RAW and its JPEG stay together.
4. **Evaluate:** `python -m lumen_train.retouch.eval --ckpt D:/lumen-train/runs/r1/best.pt --data D:/lumen-train --out D:/lumen-train/eval/r1 [--marks marks.json]` writes `report.md`/`report.json` (ΔE2000, per-panel slider error, crop IoU, angle error, each beside Auto, and the RAW-vs-JPEG gap) and `sheet.html` (before | Auto | Retouch | target) for marking. Sources the checkpoint trained on are scored on their validation split, others on every photo. Auto is its measured prior (the app's Auto without its model), measured by the `retouch_auto` example on exactly the pixels Retouch saw.
5. **Export:** `python -m lumen_train.retouch.export` saves a named model with its tiers and installs it for testing (*Model names and saving a model* below; the whole Round 12 path is *Auto v2, end to end*). Upload to `models-v1` only with Julian's OK.

The scripts build the Rust examples they call (`retouch_prep`, `render_recipe`, `retouch_auto`, `lr2lumen`) with `cargo build --release` on first use in each run, so they never use a binary older than the code.

## Adding data

One command per step; each only touches what is new (content hash in `<root>/<source>/index.jsonl`). Never point `add` at the Lumen library (it refuses).

```powershell
cd "X:/Documents/Coding/Projects/Desktop-Apps/Tauri/Photos App/scripts/ml"
$env:LUMEN_TRAIN_ROOT = "C:/lumen-train"
../../.venv-train/Scripts/python.exe -m lumen_train.data add contributor-finals "D:/Contributor/edited" --kind finals --licence "Photographer's permission for commercial training, 2026-10-07" --commercial --val 12 --test 18
../../.venv-train/Scripts/python.exe -m lumen_train.data download wikimedia --limit 3000 --contact you@example.com
../../.venv-train/Scripts/python.exe -m lumen_train.data prep
../../.venv-train/Scripts/python.exe -m lumen_train.data status
# next session, on everything prepared, starting from the last good run:
../../.venv-train/Scripts/python.exe -u -m lumen_train.retouch.train data.root=C:/lumen-train run.dir=C:/lumen-train/runs/r2 train.init=C:/lumen-train/runs/r1/best_de.pt train.epochs=60
```

`add` writes `<root>/sources.toml`; public sources stay in `datasets/registry.toml` and a name in both is an error. `download` puts photos in `<root>/originals/<source>/`; `prep` finds them there. A failed file is recorded in `index.jsonl` with its reason and counted by `status`; a deleted original leaves its prepared item and shows as *orphaned*. `--clip` is accepted but `retouch_prep` has no CLIP stage yet, so `status` shows CLIP coverage 0 %.

**Splits** are a hash of the photo's *stem* (`IMG_1234.CR3`, `IMG_1234-Edit.jpg`, `IMG_1234 copy.jpg` share `img_1234`), so adding data never moves a photo between train / val / test. Defaults val 15 %, test 2 % of the whole; the contributor's sources use `--val 12 --test 18` (about 30 test photos no run trains on). Score them with `eval --split test`.

**Which kind:** `finals` = finished photos (synthetic damage makes the pairs); `pairs` = originals + their XMP (`build_pairs`); `raw+jpeg` = an original and its edited JPEG (12-3).

`train` takes `data.sources = ["all"]` (every prepared source; only commercial ones with `--ship`) and `train.init=<ckpt>` (weights only, fresh optimiser and schedule, epoch 0; a checkpoint with another slider layout is refused).

### The contributor's photos

Two sources, both `commercial = true` with his permission (2026-10-07, `docs/THIRD-PARTY-MODELS.md`) and `--val 12 --test 18`. Splits go by stem, so a photo is on the same side in both and his test photos are never trained on in either.

- **Now, his edited JPEGs** (`finals`, synthetic damage makes the pairs): the `data add contributor-finals …` command above, then `data prep`.
- **Friday, his RAWs with the JPEGs he made from them** (`raw+jpeg`): no XMP, so `build_pairs --align` renders each RAW with Lumen's defaults, finds the JPEG in it (ORB + RANSAC on gradient images) and records his crop box and angle in the RAW's frame. The ΔE target is his JPEG warped back into the RAW's frame, with a mask of the pixels his crop kept; only the geometry sliders are known. A photo with too few matches is skipped with its reason in `<root>/contributor-pairs/pairs.jsonl`, and a RAW without a pair never trains (unedited RAWs would teach a flat render).

```powershell
../../.venv-train/Scripts/python.exe -m lumen_train.data add contributor-pairs "<folder with RAW + JPEG>" --kind raw+jpeg --licence "Photographer's permission for commercial training, 2026-10-07" --commercial --val 12 --test 18
../../.venv-train/Scripts/python.exe -m lumen_train.data prep contributor-pairs
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.build_pairs --align C:/lumen-train/contributor-pairs
```

What the pairing accepts (rehearsed on Julian's 87 NEF + edit pairs, 2026-10-07: 87 paired):

- **Edits** as JPEG, PNG or TIFF (8 or 16 bits), found by stem, so Lightroom and Photoshop endings match (`-Enhanced-NR`, `-Enhanced-SR`, `-Enhanced-RS`, `-HDR`, `-Pano`, `-Edit`, ` copy`, numbered and stacked). With several edits of one RAW, each is tried, lossless first, until one is found in the RAW.
- **Never the camera's own JPEG** (RAW+JPEG shooting): a JPEG whose EXIF names a camera model with no Software, or the camera's firmware there (`Ver.1.03`, `17.5.1`), is not an edit, whatever its name. A RAW with only that JPEG is skipped (`pairs.jsonl`: *only the camera's own JPEG*); an export beside it pairs. On Julian's library this left 83 of 87: four `Edited/` files were the D7100's JPEGs.
- **Colour profiles:** an edit tagged Adobe RGB, ProPhoto, Display P3 (any matrix/TRC profile) is converted to sRGB in float at its own depth; other profiles go through LittleCMS at 8 bits; untagged means sRGB.
- **Matching** at 1024 px, then, for a tight crop that finds too little there, again at 2048 px with the same bars (40 inliers, 30 %, scale 0.2–1.05). `pairs.jsonl` says `"pass": "fine"` for those.
- **Nikon HE / HE\* NEFs** (the Z f's, Z 8's and Z 9's High Efficiency option): no open decoder reads them (TicoRAW is licensed; LibRaw 0.22 and rawler refuse them too), so `data prep` uses **Adobe DNG Converter** when it is installed (free; it licenses the codec; `LUMEN_DNG_CONVERTER` names it if it is not in the default place), which gives the photo's own sensor data, and otherwise the full-size JPEG inside the NEF (the camera's rendering, not its sensor data). Either is written under `<root>/<source>/_converted` or `_embedded`, and the prepared folder's `stand_in.json` and `pairs.jsonl`'s `via` say which.

In training, an aligned pair knows only its geometry sliders (`target.json`'s `known`), and ΔE counts only the pixels his edit covers (`target_mask.u8` → the item's `target_void`; on Julian's 87 pairs ~30 % of the frame is outside the crop, and leaving it in doubled the untouched ΔE, 8.9 → 17.1).

### Julian's own pairs (`julian-pairs`)

`C:/lumen-train/eval-library` holds Julian's NEFs (`NEF-originals/`) and his edits (`Edited/`). They are one `raw+jpeg` source with the contributor's splits; an edit is never a photo of its own (no `target.json`, so it never trains or appears on the sheet). The script only reads the folder.

```powershell
../../.venv-train/Scripts/python.exe -m lumen_train.data add julian-pairs "C:/lumen-train/eval-library" --kind raw+jpeg --licence "Julian's own photos and edits" --commercial --val 12 --test 18
../../.venv-train/Scripts/python.exe -m lumen_train.data prep julian-pairs
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.build_pairs --align C:/lumen-train/julian-pairs
# score a run on the photos no run trains on; the sheet shows every julian-pairs test photo first (before | Auto | Retouch | edit)
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.eval --ckpt C:/lumen-train/runs/r2/best_de.pt --data C:/lumen-train --out C:/lumen-train/eval/r2-test --split test --sources julian-pairs contributor-pairs contributor-finals wikimedia-cc-by wikimedia-cc0-pd
```

`report.md` has a line per source (Retouch, Auto and untouched ΔE, crop IoU); `--sheet-all` (default `julian-pairs`) names the sources whose every scored photo is a sheet row, and `--rows` caps the others.

## Model names and saving a model

Lumen's own models are named by family, version and performance tier (Julian, 2026-10-07; `lumen_train/common/naming.py`):

| Family | For | Tiers |
|---|---|---|
| **Lumen Ray** | auto-correction (Auto) | `Lumen Ray-low`, `-high`, `-ultra` |
| **Lumen Prism** | Julian's editing style (Retouch), trained on `julian-pairs` | `Lumen Prism-low`, `-high`, `-ultra` |
| **Lumen Spectrum** | library search (CLIP, one size) | — |

A saved model is `lumen-<family>-<version>-<tier>` (`lumen-ray-1.0.1-d-high`), shown as *Lumen Ray-high 1.0.1-d*. Versions: a release is `x.y` (only when Julian says, `--release`); every build before it bumps the patch and carries a stage, `x.y.z-d|a|b|r` (dev, alpha, beta, pre-release).

**Tiers are real (Round 14-4a, `retouch/tiers.py`):**

| Tier | File | `precision` | `tta` |
|---|---|---|---|
| `low` | `-low.onnx`: static QDQ **int8** (per-channel int8 weights, uint8 activations), calibrated on up to `--calib` (64) validation inputs with CLIP missing. If int8 costs more than **0.3 ΔE** over high, it is **fp16** instead (float32 inputs/outputs) | `int8` / `fp16` | 1 |
| `high` | `-high.onnx`: the fp32 network | `fp32` | 1 |
| `ultra` | **high's file** (no copy) | `fp32` | 5: the photo, its mirror and three 90 % crops along the long side (start / centre / end), tone and colour averaged; straighten, crop and the linear/radial masks' placement come from the full pass |

Each tier's `val_de` is measured through onnxruntime on the trained sources' validation photos under `--data` (default `--root`), the way `eval` scores a checkpoint; ultra's is measured on the prepared 256 px input (crops moved to the top-left), an estimate of what Rust gets. `model.json` also records `low_vs_high_de`. Without validation photos on disk, `val_de` is left out. Save a run and install it for testing in the app (Settings → Experimental):

```powershell
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.export --ckpt C:/lumen-train/runs/<run>/best_de.pt --family ray --version 1.0.2-d --install-dev
```

It writes `C:/lumen-train/models/lumen-ray-1.0.2-d/` (the tier files, `model.json` with each tier's manifest entry, and the checkpoint for warm-starting the next version), checks the ONNX against PyTorch with and without CLIP, and refuses a run trained on a source not cleared for commercial use unless `--dev-only`. Saving one tier again (`--tiers ultra`) replaces only that tier. Installing (every file written as `*.part` and renamed, SHA-256 checked, no other model's folder touched):

```
%APPDATA%/JL-Studios/Lumen/models/dev/          ($LUMEN_DEV_MODELS overrides)
  ray/lumen-ray-1.0.1-d/model.json + -low.onnx + -high.onnx      ← --install-dev
  prism/lumen-prism-1.0.1-d/…
  retouch.json + its file                                        ← --install-dev high (old layout, also written; kept for one release)
```

**Lumen Ray 1.0** = r1 (`runs/r1/best.pt`, the 10 h 38 min Round 11B run, val ΔE 4.14), saved on 2026-10-07. **Lumen Ray 1.0.1-d** = r3ft `best_de.pt`, re-exported with real tiers on 2026-10-08: high 3.770, low fp16 3.771 (int8 cost +0.73), ultra 3.770 on 64 validation photos.

## Auto v2, end to end (Round 12-4)

Auto v2 is **Lumen Ray 1.0.1-d** (r3ft): the model behind Develop's Auto when Settings → Experimental → *Auto correction (Ray)* is on. From nothing to trying it in the app, from `scripts/ml` with `$env:LUMEN_TRAIN_ROOT = "C:/lumen-train"` (one GPU job at a time; long runs go in a script Julian starts in his own window, since a run started by a Claude session dies with it):

1. **Download** the public sources (both `commercial = true`, in `datasets/registry.toml`): `python -m lumen_train.data download wikimedia --limit 10000 --contact <email>` → `C:/lumen-train/originals/wikimedia-cc-by|cc0-pd/`. Julian's pairs are `data add julian-pairs <folder> --kind raw+jpeg` (*Julian's own pairs* above).
2. **Prepare:** `python -m lumen_train.data prep` (calls `retouch_prep`), then `data status`. On 2026-10-08: **8,760 cc-by + 3,180 cc0-pd** indexed (~11,900 photos) and `julian-pairs` 83 pairs (8 in test). `--clip` is still a no-op (no CLIP stage in `retouch_prep`), so every run trains with CLIP coverage 0 % and the app passes zeros + flag 0.
3. **Train** (`retouch.train`, `data.workers=0 data.cache=true train.val_every=2`; `run.ship=true` so only commercial sources are read; `loss.train_metric=oklab`; `synth.identity_p` stays at 0.15, *Why r1 is dark* below):
   - `r2-scratch`: from scratch, 60 epochs.
   - `r2-pre`: warm from `r1-fast`, Wikimedia only, 60 epochs (died of an OOM at epoch 21 when another GPU job ran, resumed to 60).
   - **`r3ft`**: warm from `r2-scratch/best_de.pt`, all sources (`julian-pairs` + both Wikimedia), 50 epochs, ~1 h 50.
   - `r3long`: from scratch, all sources, 100 epochs, ~3 h 40.
   The commands are in `C:/lumen-train/runs/overnight-2026-10-08.ps1` (a finished run is skipped, a dead one resumes from `last.pt`).
4. **Evaluate on the test split** (photos no run trains on), on the CPU: `python -m lumen_train.retouch.eval --ckpt C:/lumen-train/runs/<run>/best_de.pt --data C:/lumen-train --out C:/lumen-train/eval/<run>-test --split test --sources julian-pairs wikimedia-cc-by wikimedia-cc0-pd`. Mean ΔE2000 to the target (Auto = the app's Auto without its model, on the same pixels):

   | Run | All (251) | julian-pairs (8) | wikimedia-cc-by (187) | wikimedia-cc0-pd (56) |
   |---|---|---|---|---|
   | **r3ft** | **3.43** (median 3.02) | 7.37 | 3.36 | 3.09 |
   | r3long | 3.46 | 7.35 | 3.38 | 3.18 |
   | r2-scratch | 3.45 | | | |
   | r2-pre | 3.49 | | | |
   | Auto | 5.74 | 8.92 | 5.40 | 6.44 |
   | Untouched | 7.71 | 9.21 | 7.87 | 6.96 |

   r3ft's crop IoU 0.91, crop on/off right 95 %, straighten error 0.80°. Every run beats Auto on every source. 100 epochs ≈ 60 epochs ≈ a warm start: the model, not the schedule, is the limit now (a width knob is next). `julian-pairs`' test split is only 8 photos.
5. **Save and install for testing:** `python -m lumen_train.retouch.export --ckpt C:/lumen-train/runs/r3ft/best_de.pt --family ray --version 1.0.1-d --install-dev` → `C:/lumen-train/models/lumen-ray-1.0.1-d/` (low fp16 / high / ultra, sha `579aa680bd0d`) and the dev folder (`%APPDATA%/JL-Studios/Lumen/models/dev/ray/…`). Ray 1.0 (r1) stays saved beside it.
6. **Try it in Lumen:** Settings → *Experimental*: turn on *Auto correction (Ray)* (`Settings.smart.experimental.autoModel`; smart features must be on), pick *Lumen Ray 1.0.1-d* under **Model** and a tier under **Performance** (STATE.md *Experimental models*, *Round 14 app*). Develop's Auto then starts from the model's look.
7. **Mark the sheet:** `C:/lumen-train/eval/r3ft-test/sheet.html` (before | Auto | Ray | target), better / same / worse per photo. **Ship bar** (spec §3): ΔE below Auto on every source's test split (met) **and** ≥ 80 % better-or-same in Julian's marks. **Julian's marks (2026-11-01, `eval/r3ft-test/marks.json`):** 43 better, 8 same, 17 worse of 68 = **75 % better-or-same** (julian-pairs 5 / 1 / 2 of 8, 75 %; wikimedia-cc-by 38 / 7 / 15 of 60, 75 %; the sheet holds no cc0-pd photo). **The ship bar is not met** (75 % < 80 %), so Ray 1.0.1-d stays a dev build: no `--release`, no upload to `models-v1`. The next version has to win more of the photos a person looks at, not only the mean ΔE.

The contributor's sources (`contributor-finals`, `contributor-pairs`) were not on disk for these runs; the ship bar counts them once they are.

## Lumen Prism (Julian's style, Round 14)

Prism is Retouch fitted to Julian's own edits (`julian-pairs`, never the contributor's pairs): the model behind Develop's Retouch button, while Auto keeps running Ray. The kit is `lumen_train/prism/` (`config.toml` holds every setting below).

- **Warm start that never saw his pairs.** `train.init` must be a checkpoint whose `extra.sources`, and those of every checkpoint in its `config.train.init` chain, leave out `julian-pairs`; `fit.leak_free` refuses anything else (also a checkpoint that records no sources, or a missing link). Use `runs/r2-pre/best_de.pt` (chain r2-pre → r1-fast, Wikimedia only). r3ft / Ray 1.0.1-d trained on his pairs and is refused: the cross-validation would score photos it knew.
- **Partial freeze, EMA.** The encoder (`stem`, `stages`) is frozen for the first third of the epochs, then trains at 0.1 × the heads' rate (2e-4, cosine). The weights' moving average (decay 0.998) is what is scored and saved.
- **1 : 3 mix.** Each batch of 16 is 4 views of his pairs and 12 Wikimedia synthetic pairs; his rows weigh ×3 in the loss (`loss_of`'s per-row `weight`).
- **Input-only views.** Each pair gives 8 views an epoch: the photo he started from at another exposure (±1 EV), white balance (±800 K, ±10 tint) and contrast (0.8–1.2 around middle grey), mirrored half the time; his edit stays the target. A view keeps only the geometry the pair knew. No small crop yet: an aligned pair knows its crop box and ratio, which a crop would have to remap.
- **5-fold cross-validation by photo** over his non-test pairs (75 photos; a RAW's edits share a fold). Each fold scores, on its held-out photos, the warm start before training (`base`), Prism after every epoch, and Auto (the app's pre-ML Auto, `retouch_auto`, cached in `auto.json`). Prism's number per fold is its ΔE at the **median best epoch** over the folds; each fold's own best is in `prism_best` but chose its epoch on the photos it scores. The **final** model trains on every non-test pair for that median epoch count. The test split stays for the comparison with Ray 1.0.1-d.
- **Gate:** all folds finished and Prism's mean CV ΔE below the warm start's and Auto's. Then Julian marks the sheet, then it is exported as `lumen-prism-1.0.1-d` (all tiers) and, with his OK, installed in the dev folder.

Julian runs it in his own window (one GPU job at a time, ~1.5 h on the 3080 Ti):

```powershell
Start-Process pwsh -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','C:/lumen-train/runs/prism-1.ps1' -WindowStyle Minimized
# by hand, from scripts/ml (`--fold i` runs one fold; finished folds are skipped):
../../.venv-train/Scripts/python.exe -m lumen_train.prism.cv folds  data.root=C:/lumen-train train.init=C:/lumen-train/runs/r2-pre/best_de.pt run.dir=C:/lumen-train/runs/prism-1
../../.venv-train/Scripts/python.exe -m lumen_train.prism.cv final  data.root=C:/lumen-train train.init=C:/lumen-train/runs/r2-pre/best_de.pt run.dir=C:/lumen-train/runs/prism-1
../../.venv-train/Scripts/python.exe -m lumen_train.prism.cv report data.root=C:/lumen-train train.init=C:/lumen-train/runs/r2-pre/best_de.pt run.dir=C:/lumen-train/runs/prism-1
# the test split beside Ray 1.0.1-d's (the sheet shows every julian-pairs test photo):
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.eval --ckpt C:/lumen-train/runs/prism-1/final/best_de.pt --data C:/lumen-train --out C:/lumen-train/eval/prism-1-test --split test --sources julian-pairs --sheet-all julian-pairs
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.eval --ckpt C:/lumen-train/runs/r3ft/best_de.pt --data C:/lumen-train --out C:/lumen-train/eval/ray-1.0.1-d-pairs-test --split test --sources julian-pairs --sheet-all julian-pairs
# after the gate and his marks:
../../.venv-train/Scripts/python.exe -m lumen_train.retouch.export --ckpt C:/lumen-train/runs/prism-1/final/best_de.pt --family prism --version 1.0.1-d --root C:/lumen-train
```

**prism-1** (2026-10-08, warm start `r2-pre`): 5-fold CV ΔE **8.37 ± 1.23** against the warm start's 8.82 and Auto's 13.07, gate passed (median best epoch 14; fold 0 overfits: 10.09 against its warm start's 8.78, best at epoch 2). On the 8 test photos the final fit scores **7.06** (median 5.34) against Ray 1.0.1-d's 7.37 (5.80), Auto 8.92 and untouched 9.21; crop on/off right 75 % against 50 %. Exported as **Lumen Prism 1.0.1-d** (`C:/lumen-train/models/lumen-prism-1.0.1-d/`): high 3.762, low **int8** 3.973 (+0.21, under the 0.3 fp16 bar), ultra 3.775.

`auto.json` comes from `retouch_auto`, which `eval.rust_auto` builds in the kit's own `src-tauri` (a full release build when the kit runs from a worktree); `prism-1/auto.json` was made with main's built example. On the non-test pairs Auto scores ΔE 13.1 against 9.1 untouched (worse on 60 of 75); the test split's 8.92 is reproduced exactly, so the warm start is the bar that matters. A model trained on his pairs is for his use: its `licence` reads `trained on julian-pairs (Julian's own photos and edits), …` and it is never shipped without his say (THIRD-PARTY-MODELS.md). Its tiers' `val_de` in `model.json` is measured on validation photos, which Prism's final fit trained on: it compares Prism's tiers with each other, never Prism with Ray (that is the CV and the test split).

## Speed

r1's configuration (484 train / 74 val, batch 64, 240 epochs) trains in **24 minutes** on the RTX 3080 Ti (it took 10 h 38 min in Round 11B) and reaches val ΔE2000 3.98 (`r1-fast`). What made the difference:

- **Synthetic damage on the GPU.** The dataset hands over the finished photo and the damage it drew; `synth.realise` turns the whole batch into pairs on the device (same seed, same damage as before). `data.cache=true` keeps decoded photos in RAM (float16, ~0.4 MB each; use it with `data.workers=0`, since each worker would keep its own copy).
- **Under 12 GB.** The loss's render keeps only each stage's output and recomputes the rest in the backward pass (`render.render(..., recompute=True)`, same values). Without that a batch of 64 needed ~13 GB, Windows spilled it to system memory, and every step took ~8× longer. If you raise the batch size, watch the bench's peak VRAM and keep it below the card's memory.
- **Fewer kernels.** Channel curves, hue bands and grading run as one batched op each.
- `loss.train_metric = "oklab"` trains on a cheaper OKLab distance and switches back to CIEDE2000 for the last `loss.de2000_tail` (10 %) of the epochs. It now saves only ~10 % of a step. Validation always scores CIEDE2000. `train.val_every = n` validates every n epochs (and always after the last).
- Each run keeps `best.pt` (lowest validation loss), `best_de.pt` (lowest validation ΔE2000, the one to start the next run from or export) and `last.pt`.

To see where a step spends its time: `python -m lumen_train.retouch.bench data.root=C:/lumen-train data.workers=0 data.cache=true steps=10` (top CUDA and CPU ops, kernels per step, peak VRAM). Profiler overhead inflates its step time; for absolute numbers, read the `train` and `epoch` times in the training log.

## Why r1 is dark and never crops (2026-10-31)

The in-app check of Ray 1.0 (r1) showed exposure −0.2 to −0.5 on photos that needed nothing and never a crop (STATE.md). Both come from r1's training, not from the app.

**Exposure bias: no untouched samples.** Probe: 60 Wikimedia test photos, damaged by a known exposure e ∈ {−1, −0.5, 0, +0.5, +1}; a perfect model answers −e. On the undamaged photo r1 and r1-fast answer about −0.37 EV; r2-scratch, r2-pre, r3long and r3ft (Ray 1.0.1-d) answer about 0. Two things changed between them: the data (~560 → ~12,000 photos) and `synth.identity_p = 0.15` (commit 14ac9fb: 15 % of samples are the finished photo itself, target all zeros). The ablation separates them: same config as r2-scratch, Wikimedia only, 12 epochs from scratch, only `identity_p` differs (`runs/abl-idp0`, `runs/abl-idp015`):

| damage | idp 0 answer (offset) | idp 0.15 answer (offset) |
|---|---|---|
| −1.0 | +0.23 (−0.77) | +0.42 (−0.58) |
| −0.5 | −0.04 (−0.54) | +0.17 (−0.33) |
| 0 | −0.32 (−0.32) | +0.00 (+0.00) |
| +0.5 | −0.68 (−0.18) | −0.45 (+0.05) |
| +1.0 | −1.14 (−0.14) | −1.00 (+0.00) |

Val ΔE2000 is the same (3.85 vs 3.83), so the score doesn't show it. Without untouched samples, a photo that needs nothing gets darkened on 12,000 photos too: the cause is `identity_p = 0`, not r1's small data. The damage is symmetric (exposure ~ N(0, 0.8), clamped to ±2), and the answer barely follows the photo's clipped share or brightness (corr −0.3), so it's a learned offset, not a reaction to highlights. Why the offset is negative rather than positive isn't tested. **Keep `identity_p > 0`.** To check a new run: `python -m lumen_train.retouch.probe_exposure --data C:/lumen-train runs/<run>/best_de.pt` (offset at damage 0 within ±0.05).

**Still open: dark photos are half-corrected.** Even with identity samples, a photo 1 EV too dark gets +0.42 (Ray 1.0.1-d: +0.43, offset at 0 −0.003), while over-bright photos are corrected fully. Guess (not tested): a darkened photo looks like one that is meant to be dark (night, low key); a brightened one clips, which is unambiguous.

**Crop head: too few photos.** r1 trained on 484 photos. Its validation crop on/off loss bottoms at 0.51 and climbs to 1.18 (always answering "no crop" scores 0.56), and the crop IoU never improves: it overfits before learning anything. r1-fast has the same curve; on 10,472 photos (r2-scratch) the same head improves steadily. Ray 1.0.1-d's crops are its own question; r1's crop head is not worth fixing.

## Adding the next model

Make `lumen_train/<model>/` with its own `model.py`, `train.py`, `config.toml`, and use `common/`:
- `common.config.load(path, overrides)` reads TOML with `a.b=value` overrides.
- `common.device.pick_device()` and `seed_everything(seed)`.
- `common.logs.Logger(dir)` writes `log.csv` and, if installed, TensorBoard.
- `common.ckpt.save/load` keep the model, optimiser, epoch, best metric and the sources seen.
- `common.licence.check(sources, ship)` is the licence gate.
- `common.onnx_export.export(model, example, path, inputs, outputs)` exports, checks parity and hashes. `common.manifest.entry(...)` makes the `models.json` entry.

Anything the app must compute the same way (inputs, layouts, maths) is written by a Rust test into a JSON file and read by the scripts, never copied by hand.
