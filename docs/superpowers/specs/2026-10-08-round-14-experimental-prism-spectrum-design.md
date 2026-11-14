# Round 14 — Experimental tab, real tiers, Prism, Spectrum (design, 2026-10-08)

Approved by Julian in chat on 2026-10-08. Round 13 (OCR, spec on branch `round-13-ocr`) moves after this round; its migration number (0031) stays free — **Round 14 needs no migration**.

## Why

- Julian wants one place to choose, per AI task, which trained model runs and how heavy it runs, with the app telling him when his PC is too slow for a choice.
- The tiers are fake today: `lumen-ray-1.0.1-d-{low,high,ultra}.onnx` are byte-identical (sha `579aa680…`), and only `high` is installed in the dev folder (`retouch.json` names one model), so the app can only ever show *high*.
- Retouch should be greyed out until its own model exists: **Lumen Prism**, trained on Julian's own RAW + edit pairs to edit like he does.
- Search (**Lumen Spectrum**) was never tuned on real queries (Round 4K Task 3 waits on Julian's marks).

## What Julian said (2026-10-08)

- "Finish the search engine" = **Spectrum** (CLIP search, Round 4K Task 3), not OCR.
- **Prism = his editing style**: the Retouch model learned from his ~83 pairs; optimise for quality on little data.
- **Retouch is disabled with an *In development* badge** (Develop button, key, palette, Experimental row) until a Prism model is installed; then it unlocks by itself.
- **Performance is chosen by benchmarking each model on this machine**, not by hardware rules. Ultra should be reachable on low/mid PCs when the model is light; his RTX 3080 Ti must not be stuck on High.

## §1 Real tiers (made at export, same weights family)

| Tier | File | How it runs | Expected |
|---|---|---|---|
| `low` | int8 ONNX (`onnxruntime.quantization`, static QDQ, calibrated on ~64 validation inputs). If int8 costs more than **0.3 ΔE** over `high` on the validation set, `low` is fp16 instead and `model.json` says so | one pass | fastest, smallest |
| `high` | today's fp32 ONNX | one pass | reference |
| `ultra` | the **same file as `high`** (not a copy: `file` names the high file) | `tta: 5` — Rust runs the image, its horizontal mirror and 3 centre-shifted crops (90 % of the frame, left/centre/right along the long side) and averages the **tone/colour** outputs; **geometry outputs (crop, straighten) come from the unmirrored full-frame pass only** | steadier, ~5× compute |

A later wider network simply becomes the `ultra` file with `tta: 1`; nothing else changes. Export records each tier's validation ΔE (`val_de`) so the UI can show the quality difference honestly.

## §2 The dev catalogue (contract between the Python and Rust sessions)

```
config_dir/JL-Studios/Lumen/models/dev/          ($LUMEN_DEV_MODELS overrides)
  ray/lumen-ray-1.0.1-d/model.json
  ray/lumen-ray-1.0.1-d/lumen-ray-1.0.1-d-low.onnx
  ray/lumen-ray-1.0.1-d/lumen-ray-1.0.1-d-high.onnx
  ray/lumen-ray-1.0/…
  prism/lumen-prism-1.0.1-d/…
  retouch.json + its file                         (legacy; still read as one Ray model, tier "high")
```

`model.json` is the file `export.py` already writes to `C:/lumen-train/models/<id>/model.json` (`name`, `family`, `version`, `purpose`, `tiers.{low,high,ultra}` each a `ModelEntry` with `id`, `name`, `role: "retouch"`, `file`, `bytes`, `sha256`, `licence`, `retouch{size,layout,inputs,output}`, `trained{…}`), plus per tier:

- `precision`: `"int8" | "fp16" | "fp32"`
- `tta`: `1` or `5`
- `val_de`: the tier's own validation ΔE (may be absent on old models)

Tiers may share a `file` (ultra → high); the sha check runs once per file. A tier whose file is missing or fails its sha is that tier's state (`invalid`), never an error. `export.py --install-dev` copies **every** tier into `dev/<family>/<id>/` and never deletes another model's folder. `--install-dev <tier>` (old flag form) keeps working for one release of the kit.

**Task → family:** `auto` → `ray`, `retouch` → `prism`, `search` → `spectrum` (built-in, from the `models-v1` manifest: *Lumen Spectrum 1.0 (based on CLIP ViT-B/32)*, one size), `text` → `glyph`, `masks` → `facet` (none installed yet: rows greyed).

## §3 Performance detector

- `src-tauri/src/model_bench.rs`: for each installed model × tier, on the maintenance lane at below-normal priority, after start-up has settled: build the session the app would use (same execution provider and `settings.smart.gpu`), 1 warm-up + 5 timed runs on a synthetic input of the model's `size` (TTA counted: an ultra run is the full 5-pass cost), keep the median ms.
- Cache: `config_dir/JL-Studios/Lumen/model-bench.json`, keyed by `sha256 + tier + fingerprint`. Fingerprint = OS, CPU brand, logical cores, RAM (GiB), GPU adapter name and the execution provider in use. A new model, a new GPU/driver-visible adapter, or a GPU on/off switch re-times only what changed. A *Re-test* button forces it.
- Budgets (constants, tuned once on Julian's PC and noted in STATE.md): **Auto ≤ 1000 ms**, **Retouch ≤ 1500 ms** of model time per call.
- `recommended(task)` = the highest tier (ultra > high > low) whose median fits the budget; if none fits, `low`. Not yet timed → `high` and the row says *Testing this PC…*.
- A manual tier over budget is allowed and shows a warning: *Ultra takes about 4.1 s here; Lumen recommends High.*
- Commands: `experimental_catalog() -> Vec<ExperimentalTask>` (task, family, installed models with their tiers' states, timings and the recommendation), `retest_models()`. ts-rs types generated.

## §4 Settings → Experimental (its own tab)

- New top-level Settings section **Experimental** (after Smart features; the old *Experimental* group under Smart features moves here). Off unless Smart features is on.
- One row per task — **Auto correction (Ray)**, **Retouch (Prism)**, **Search (Spectrum)**, **Text in photos (Glyph)**, **AI masks (Facet)**:
  - on/off switch (where it applies: Auto, Retouch);
  - **Model** select: installed models newest first; a family with nothing installed shows the select disabled, *No model yet*;
  - **Performance** select: *Automatic (High)* — the recommendation in brackets — then Low / High / Ultra, each with its measured time (*Ultra · 180 ms*) and ΔE where known; Spectrum shows *One size*;
  - warning line when the pick is over budget; status line from today's `experimentalLine`.
- **Retouch** row, Develop's Retouch button, its key and its palette entry are **disabled with an *In development* badge while no Prism model is installed**. With a Prism model they behave as today, running Prism. Auto keeps running Ray.
- Settings shape (`photos-core::settings`, `#[serde(default)]`, old files load): `smart.experimental` gains `tasks: { auto: TaskChoice, retouch: TaskChoice }` with `TaskChoice { model: Option<String> /* model id, None = newest */, tier: "auto" | "low" | "high" | "ultra" }`. The existing `auto_model` / `retouch` booleans stay the on/off switches.

## §5 Lumen Prism 1.0.1-d (Julian's style, few-shot)

Data: source `julian-pairs` (83 aligned pairs) only for the target style, with Wikimedia sources as a regulariser. **Never the contributor's pairs** (that is Ray's/Creative Retouch's data).

- **Warm start from a checkpoint that never saw `julian-pairs`** (check `data.sources` in the candidate's saved config: r2-scratch or r2-pre are expected to be Wikimedia-only; r3ft/Ray 1.0.1-d trained on julian-pairs, so warm-starting from it would leak into the cross-validation). If none qualifies, train a Wikimedia-only base first (`r2`-style, ~25 min with 12-1's speed-ups).
- **Partial freeze:** encoder frozen for the first third of training, then unfrozen at 0.1× LR; heads train at full LR. Low base LR, cosine schedule, EMA of weights.
- **Mix:** each batch is 1 julian-pair : 3 Wikimedia samples (the julian-pairs loss weighted ×3), so the style dominates without overfitting 83 photos.
- **Input-only augmentation for the pairs:** random exposure (±1 EV), white balance (±800 K, ±10 tint), contrast and a small crop/flip applied to the **input** while the target stays Julian's edit (geometry augmentations applied to both consistently), so the model learns to reach his look from any starting point. Many augmented views per pair per epoch.
- **Evaluation: 5-fold cross-validation over julian-pairs grouped by stem** (folds never split a photo), mean ± std ΔE per fold, against Auto and against the warm-start base on the same folds. Plus the existing `julian-pairs` test split vs Ray 1.0.1-d. The final model trains on all folds' data for the CV-median best epoch count.
- **Gate:** Prism is exported (stage `-d`, `lumen-prism-1.0.1-d`, all tiers per §1) only if its CV ΔE beats the base and Auto on julian-pairs; Julian marks its sheet (`eval --sheet-all julian-pairs`) before it is installed. A model trained on Julian's photos is for his use; `licence` says so (`trained on julian-pairs (Julian's own photos and edits)`), and THIRD-PARTY-MODELS/LICENSING docs note it is not shipped without his say.
- Julian runs the real training himself in his own terminal (one GPU job at a time; don't run it from a Claude session that may close).

## §6 Spectrum: Round 4K Task 3 finished

Per `docs/superpowers/plans/2026-09-26-round-4k-search.md` Task 3, using the existing harness `src-tauri/src/search_eval.rs` on a **library copy in the scratchpad** (never `library.db` itself):

1. Ask Julian for ~20 queries he'd really type; write contact sheets; he marks wrong results; marks saved as JSON in the scratchpad.
2. Measure precision@20 and recall for today's rules, then (a) RRF of FTS and semantic ranks (k = 60), (b) query-adaptive floor, (c) prompt ensembling for the text tower. Keep what raises precision@20 without cutting recall; write the table into the 4K plan.
3. *Best match* ordered by the fused score (keyset, no `OFFSET`), *Newest first* toggle unchanged.
4. Query-to-first-page stays ≤ 250 ms on the real library copy (prompt ensembling costs 2–3 text-tower runs: cache them).

## Phases and sessions

| Phase | Session | Branch / worktree | Owns |
|---|---|---|---|
| 14-1 catalogue reader + tiers in Rust (TTA, precision, legacy `retouch.json`) | **S1** | `round-14-app` | `src-tauri/src/{models.rs, develop_retouch.rs, develop_auto.rs, model_bench.rs}`, `photos-ml::retouch`, `photos-core::settings`, `commands/develop.rs` (catalogue commands) |
| 14-2 benchmarks + detector | **S1** | same | same + `src-tauri/src/maintenance.rs` (one lane entry) |
| 14-3 Experimental tab + Retouch greyed | **S1** | same | `src/routes/settings/**`, Develop's Retouch button/key/palette entry, `src/lib/menus/` entries for it, generated types |
| 14-4a real tiers + catalogue install in export | **S2** | `round-14-prism` | `scripts/ml/**`, `docs/ML-TRAINING.md` |
| 14-4b Prism few-shot kit, CV, gate, export | **S2** | same | same, `docs/THIRD-PARTY-MODELS.md` |
| 14-5 Spectrum tuned | **S3** | `round-14-spectrum` | `src-tauri/src/{search.rs, search_eval.rs}`, `photos-core::{embedding, layout, query, search_overview}`, `src/routes/search/**`, the 4K plan |

Each session writes its own phase plan(s) under `docs/superpowers/plans/2026-10-08-round-14-<n>-*.md` first, keeps to the ~70k budget per session (hand-off line in its plan, then stop), and merges into `main` only with Julian's OK. A file another session owns is not touched: note it in the hand-off. Shared docs (`docs/STATE.md`, `CLAUDE.md`) are updated once per branch, at merge.

## Testing

- Rust: catalogue parsing (new layout, legacy `retouch.json`, shared files, bad sha → tier invalid), TTA averaging with a fixture ONNX (mirror un-done; geometry from pass 1), bench cache keyed by fingerprint, `recommended` rules, settings back-compat.
- Python: int8 export within the ΔE bound on a tiny fixture model, `install_dev` layout, CV fold grouping by stem, input-only augmentation keeps the target untouched.
- UI: Vitest for the Experimental rows (greyed families, *In development*, warning over budget, Automatic shows the recommendation), Retouch disabled in Develop and the palette without Prism.
- Spectrum: fixture tests for RRF order, adaptive floor, best match as a layout order; real-copy timings.
