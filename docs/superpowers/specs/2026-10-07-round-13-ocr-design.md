# Round 13 — Text in photos (OCR), Latin first (2026-10-07)

Brainstormed with Julian on 2026-10-07 (Round 12's lane C hand-off: "then the OCR spec"). Design approved in chat section by section (§1–§4); answers recorded under *Decisions*. Plans follow as `docs/superpowers/plans/2026-10-07-round-13-<n>-*.md`, one per phase in §5.

## Why

Screenshots, signs, menus, tickets and documents are a large share of a phone library, and today their words are invisible to search. Round 13 reads the text in every photo on the device, makes it searchable, and lets the user see, select and copy it — Latin script with diacritics first, done well and fast, before any other script.

## Decisions (Julian, 2026-10-07)
- **Round 13 is Latin + diacritics only** (Slovenian, Croatian, German, French, Spanish, Italian, Polish, Czech, English…: č ć š ž đ ä ö ü ß é…). It is trained (§4) and every feature below is finished before any other script.
- **Later, after 8f masks and pet identity**, the OCR track continues in this order (§6): more scripts (all Cyrillic alphabets, Greek, Japanese, Chinese) with a script filter → language identification (regional, several candidates, user corrections, a language filter) → translation.
- What the user can do with the text: **search, the list in the viewer's info with Copy, a selectable Live Text–style overlay, and a *Has text* filter.**
- Scanning: the background pass **plus scan on open**; stills only (no video frames, no special ordering for screenshots). **Built for speed from the start** (budgets in §2).
- Recognisers per line, when several scripts arrive (step 2): **run all, keep the best, index a close second** (approach A in the brainstorm). Round 13 has one recogniser and no routing.
- Branch `round-13-ocr` (worktree `.claude/worktrees/round-13-ocr`).

## Constraints
- CLAUDE.md's, especially: originals are never read for display or scanning when a cached derivative exists, never written; tests and replays run on copies; missing models are a state, never an error.
- Migration **0031** is this round's (the next free is 0032 afterwards).
- Every model, font, corpus and background must allow commercial use (`docs/LICENSING-AUDIT.md`, `docs/THIRD-PARTY-MODELS.md`); training data sources are marked `commercial = true` or refused by the licence gate.
- Training never reads Julian's library; evaluation photos come from `C:/lumen-train/eval-library/` (a folder he copies into, read-only).
- The feature sits behind Round 12's **Experimental** switch (12-5) and loads models from `config_dir/JL-Studios/Lumen/models/dev/` until it ships in a models release. Round 13 starts after Round 12's lane B (12-5) is merged.
- `photos-ml` depends only on `photos-core`; nothing before Phase 7 depends on it.

## §1 Storage and search

**Migration 0031**

```sql
CREATE TABLE ocr_scan (                 -- like face_scan / pet_scan
  asset_id INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  model    TEXT NOT NULL                -- "<det id>+<rec id>"; a new model rescans
);
CREATE TABLE asset_text (               -- one row per line read
  id       INTEGER PRIMARY KEY,
  asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  ord      INTEGER NOT NULL,            -- reading order
  text     TEXT NOT NULL,
  conf     REAL NOT NULL,
  quad     BLOB NOT NULL,               -- 4 corners × (x, y) f32, 0..1 of the unedited photo (as faces)
  script   TEXT NOT NULL,               -- ISO 15924: 'Latn' now; step 2 adds Cyrl/Grek/Jpan/Hans
  lang     TEXT,                        -- NULL until step 3
  alt      TEXT                         -- step 2's close second reading; search only, never shown
);
CREATE INDEX idx_asset_text_asset ON asset_text(asset_id, ord);
```

`assets_fts` is recreated with a fifth column, **`photo_text`**, and refilled from the catalogue as 0004 did (FTS5 cannot add a column). 13-2 measures the refill at 250 k rows; if it is too slow for a migration, the migration creates the table empty and `fts::rebuild` runs on the Maintenance lane, with search falling back to `LIKE` on `filename` until it finishes.

- **One index, not two:** FTS5 ANDs across a row's columns, so "Anna menu" finds a photo where Anna is a tagged person and *menu* is on a sign. A separate text index could not.
- `fts.rs` still owns every write: its row query gains `photo_text = group_concat(text || ' ' || COALESCE(alt, ''), ' ')` ordered by `ord`; the OCR job calls `fts::index_ids` once per batch.
- **Noise:** lines under a confidence threshold (start 0.6, tuned on the eval set in 13-5) and single-character lines are not stored.
- **Diacritics:** the tokenizer (`unicode61 remove_diacritics 2`) folds accents, so *cevapcici* finds *ćevapčići* and back. A test covers č ć š ž đ; **đ/Đ has no combining mark** and may not fold — if the test shows that, `fts.rs` maps đ→d, Đ→D on both the indexed text and the query.
- **Filter:** `AssetQuery.has_text: bool` → `EXISTS (SELECT 1 FROM asset_text t WHERE t.asset_id = a.id)`; smart albums get the same rule.
- **Switch off:** search leaves out `photo_text` with an FTS5 column filter (`{filename device_name place_name keywords} : …`) in `search.rs`; the rows stay.

## §2 Pipeline and job

**`photos-ml::ocr`** (new module)
- `TextDetector` — PP-OCRv5 mobile detection (DB). Preprocessing exactly as PaddleOCR: sides to multiples of 32, ImageNet mean/std. Postprocessing in Rust: binary threshold on the probability map → contours → minimum-area rotated box → box score → unclip (Vatti offset by `area · ratio / perimeter`) → quad in image pixels.
- `TextRecognizer` — `latin_PP-OCRv5_mobile_rec` (CTC) at 48 px height with its `ppocr_keys_latin.txt`. Each quad is perspective-warped upright (rotated 90° when taller than 1.5× its width, as PaddleOCR does); greedy CTC decoding; line confidence = mean of the kept characters' probabilities.
- **Golden test vs PaddleOCR (Python)**, the method used for the face models vs OpenCV: a public-domain fixture with signs; boxes within ~1 px, text identical. `scripts/ml/export_13.py` exports the ONNX files and the reference output; it only adds files, never re-exports earlier rounds'.
- Model entries in the manifest as for faces/pets; the scan key is `"<det id>+<rec id>"`.

**What the model sees**
- The 1536 px scan image from `derivatives::decode_for_scan` (cut in memory from the cache, as the face scan), never the original when a derivative exists.
- **Detection at 736 px on the long side** (was 960; changed 2026-11-01, Julian's choice, because 960 missed the budget below: 115.8 ms detection, 136.5 ms without text, 13.6 min per 5k); **screenshots** (`AssetKind::Screenshot`) at 1536, their text being small and dense. **The budget is met at 736 px on 4 intra-op threads** (detection 57.3 ms, no-text photo 70.4 ms, 5,000 photos ≈ 7.1 min; quiet Ryzen 9 5900X, 2026-10-31), at the cost of fewer lines found (54 of the 75 found at 960 on the eval set). The detection size is part of the scan key.
- Recognition crops come from the 1536 px image, so small text keeps its pixels.
- **No box → no recognition**: the common case ends after detection.
- Recognition sorts crops by aspect ratio and runs batches of up to 16, padded only to the batch's widest.

**`OcrEngine`** — detector + recogniser, 4 intra-op threads each (2 until 2026-11-01), opened lazily, kept in `AppState` behind a mutex, shared by the background job and scan on open (≈ 15 MB of models).

**Background job** (`src-tauri/src/ocr.rs`)
- Maintenance lane, after the face and pet scans; newest first; batches of 16; *Reading text · N left* in the activity ring (`ocr://progress`).
- Runs only when the Experimental switch is on and the models are present; starts after imports, on the six-hour tick, when the switch turns on and when models arrive.
- Rescans an asset whose `ocr_scan.model` differs from the current key.
- Reuses the face scan's runner (`faces::run` / `Scanner`) if it is general enough; 13-2 checks before writing another.

**Scan on open**
- `read_text(asset_id) -> Vec<TextLine>` (`#[tauri::command(async)]`): stored lines when the asset is scanned with the current key; otherwise scans on the **Interactive** lane under job key `ocr:{id}` (so it never runs twice beside the background job), writes, reindexes FTS for that asset, returns.
- The viewer calls it for the open photo; its two neighbours go on the Prefetch lane.

**Failures**: missing models → the feature is off (a state). An asset that will not decode is recorded in `ocr_scan` with no lines and its cause logged, so the job never loops on it.

**Budgets** (an `--ignored` benchmark over the eval set, release build, this machine's CPU):

| Measure | Budget |
|---|---|
| Detection, median per photo | ≤ 60 ms |
| Photo without text, decode included | ≤ 80 ms |
| Scan on open, until lines are returned | ≤ 400 ms |
| 5,000 photos in the background | < 10 min |
| `assets_fts` refill at 250 k rows | measured in 13-2 (decides migration vs Maintenance lane) |

## §3 Interface

**Experimental switch** (Settings → Smart features → Experimental, from 12-5): *Read text in photos*, off by default. On: the job starts; viewer, filter and search use the text. Off: nothing new is scanned, the viewer and filter hide text, search leaves out `photo_text`; rows are kept so turning it back on is instant.

**Viewer — info panel**: a *Text* section when the photo has lines: the lines in reading order, selectable; *Copy all*; *Copy* on a line's context menu. While a scan on open runs it says *Reading text…* inline (no spinner).

**Viewer — overlay** (Live Text–style)
- A *Text* button in the viewer toolbar, **only when the photo has lines**. It (or a key the plan picks after checking the app-wide keys) dims the photo slightly and outlines each line.
- Each line is a **real text element** (transparent glyphs) positioned and skewed onto its quad inside the zoom/pan layer, so drag-select, `Ctrl/Cmd C`, double-click a word and screen readers work natively — no custom selection code.
- Quads are stored on the unedited photo and go through the **same mapping as face boxes** to follow the edit's crop and rotation.
- Context menu (factory in `src/lib/menus/`): *Copy*, *Copy all*, *Search for "…"*. `Esc` or the button turns it off. The overlay fades in and out as a whole; never per line.

**Filter**: a *Has text* chip beside the existing chips, the smart-album rule, a palette entry. Empty state: *No text found in these photos yet* — action *Turn on reading text* when the switch is off; *Still reading text · N left* while the job runs.

**Palette**: *Show text in photo*, *Copy text from photo*, *Filter: has text*.

**Styling**: the outline and dim colours are tokens in `globals.scss`; nothing branches on theme.

## §4 Training and evaluation

**Kit `scripts/ml/lumen_train/ocr`** (beside `retouch`)
- **Synthetic lines:** fonts under the SIL Open Font License (sans, serif, mono, condensed, display, handwriting-like; rendered for training, never redistributed); backgrounds = CC0 photo crops plus flat UI colours for screenshot-like lines; augmentations: perspective, slight curve, blur, noise, JPEG, lighting/shadow, low contrast.
- **Text:** public-domain Project Gutenberg books in Slovenian, Croatian, German, French, Spanish, Italian, Polish, Czech and English; generated sign/screenshot strings (prices, dates, times, URLs, opening hours, all-caps); a deliberate surplus of č ć š ž đ ä ö ü ß é è ñ ł ő ű. No Wikipedia (CC BY-SA on trained weights is unclear).
- **Training:** fine-tunes the Latin recogniser only; the detector stays stock. Reuses 12-1's speed work (GPU batches, `val_every`, best checkpoint by the metric). Sources and fonts recorded in `docs/THIRD-PARTY-MODELS.md` / `LICENSING-AUDIT.md`, each `commercial = true`.

**Evaluation** (`python -m lumen_train.ocr.eval`)
- ~200 labelled lines from screenshots and signs in `C:/lumen-train/eval-library/`; labels pre-filled by the stock model, corrected by hand (Julian or a session viewing the crops).
- Measures: **character error rate**; **search recall** (share of the real words of ≥ 3 letters a search finds, accents folded); detection precision and recall.
- **Ship bar:** the fine-tuned recogniser installs to `models/dev/` only if it has a lower character error rate **and** search recall no worse than stock, overall and on English alone. Otherwise stock ships.

## §5 Phases (each ≤ ~70k tokens)

| Phase | What | Owns | Depends on |
|---|---|---|---|
| 13-1 | Vet licences, `export_13.py`, `photos-ml::ocr` (detector, recogniser, postprocessing), golden test | `photos-ml/src/ocr.rs`, `scripts/ml/export_13.py`, `docs/THIRD-PARTY-MODELS.md` | — |
| 13-2 | Migration 0031, `fts.rs` column and đ test, `has_text`, the job, `read_text`, budget benchmark | `photos-core::{fts,query,ocr}`, `src-tauri/src/ocr.rs`, `commands/ocr.rs` | 13-1 |
| 13-3 | Switch, info section, overlay, filter chip, palette, empty states | viewer, filters, Settings → Experimental, `src/lib/menus/` | 13-2 |
| 13-4 | Synthetic kit, eval set and harness | `scripts/ml/lumen_train/ocr/**` | — (parallel lane) |
| 13-5 | Fine-tune, evaluate against the bar, install to `models/dev/` if it passes; `docs/ML-TRAINING.md` | the kit, runs | 13-1, 13-4 |

**Testing:** TDD for the Rust parts (box postprocessing, unclip, CTC decoding, quad mapping, migration and FTS round-trips, the đ test, `has_text`); Vitest for the overlay, the info section and the chip; the golden test and the budget benchmark (`--ignored`, `LUMEN_MODELS_DIR`).

## §6 Later (after 8f masks and pet identity; each gets its own spec)
1. **More scripts** — all Cyrillic alphabets (Russian, Serbian, Macedonian, Bulgarian, Ukrainian…), Greek, Japanese, Chinese. PP-OCRv5 ships `cyrillic`, `eslav`, `el` and CJK recognisers. Lines routed by approach A: every recogniser reads every line, the best is shown, a reading within a small margin is indexed in `alt` (Latin/Cyrillic/Greek look-alikes: "РЕСТОРАН" vs "PECTOPAH"). A **text-script filter** (Latin / Cyrillic / Greek / Japanese / Chinese) from `asset_text.script` — no model needed. Japanese and Chinese have no spaces, so they get a second FTS5 table with the `trigram` tokenizer. Later, the kit may train one recogniser with a merged Latin + Cyrillic + Greek dictionary, replacing three only if it beats them.
2. **Language identification** — a regional language per text block (Slovenian, Serbian (Cyrillic), Serbian (Latin)…). When the evidence fits several (Croatian / Bosnian / Serbian Latin), all are shown. The user can **reject** a language or **add** a missing one, like People corrections, and corrections become evidence. Julian corrects only once the model is well trained. A **language filter** beside the script filter. Fills `asset_text.lang` (one language per line, or a side table if several per line are kept — decided in that spec).
3. **Translation** on the go from the viewer's text. **Open:** online translation (Google Cloud Translation with the user's own key, opt-in and clearly marked online) conflicts with "no cloud" and is paid; on-device models (Firefox's Bergamot / Opus-MT, commercial-friendly) keep it offline. Decided when that step is brainstormed.

## Out of scope for Round 13
Video frames, other scripts, language identification, translation, handwriting as a goal (only what the stock/fine-tuned Latin model reads), document scanning (deskew/crop to page), PDF export of text.
