# Lumen — On-device ML (Phase 7) Design

**Status:** approved scope 2026-09-22 · **Owner:** JL Studios · **Spine:** `2026-09-17-photos-app-design.md` (§4.3 crates, §7 jobs, §13 memories, §14 slideshows)

Phase 7 adds models to Lumen. It is two tracks that share one runtime and one set of image embeddings:

| Plan | Delivers |
|---|---|
| **7a** — runtime, embeddings, creative algorithms | `photos-ml`, automatic model download, a CLIP-class embedding per photo, a beat tracker that follows the music, and five algorithms that use them: **cut to the beat**, **music selection**, **memory photo selection**, **transition selection**, **slideshow grouping**. |
| **7b** — people and search | face detection and embeddings, people clustering and naming, scene tags into `assets_fts.keywords`, natural-language search by embedding similarity. |

7a comes first (decided 2026-09-22). This document fixes the decisions for both; plan 7a implements §1–§9, plan 7b §10.

---

## 1. Why each algorithm changes — what is wrong today

Measured on the dev machine, 2026-09-22.

- **Cut to the beat does nothing on real music.** `photos-media::beats::estimate` computes *one* tempo and *one* phase for a whole track by autocorrelating a broadband loudness-rise envelope. On the saved Jamendo track *What Is Love* (212 s) it returned 90.7 BPM at confidence **0.16**, below `MIN_CONFIDENCE` 0.25, so `sync_slideshow_to_music` answered `no_beat` and left the show alone. Twenty-second windows of the same track disagreed: 89–96 BPM, with octave errors at 61.5 and 191.4, and phases ±300 ms off one grid (a beat is 662 ms). Even above the threshold the cuts would drift: tempo is quantised to whole 11.6 ms hops, so the click tests read 90 → 90.7, and 0.7 BPM of error is 0.47 s of drift after a minute.
- **Music selection** knows a memory only by its *kind* (`mood_for_memory(kind)`): every trip gets the same mood, whatever the pictures show.
- **Memory curation** judges a photograph by favourite, stars and sharpness, and repetition by time, place and a 64-bit dHash. It cannot tell a sunset from a receipt, or two different frames of the same child from two different subjects.
- **Transition selection** is `Crossfade` everywhere, or a seeded random draw (`Random`). It never looks at the pictures or the music.
- **Slideshow grouping** puts photographs in one grid purely because they are adjacent and the same shape, so a grid can hold a landscape of a beach and a close-up of a menu.

(The preview's broken transitions — squeeze, reveal, circle-close, corner, zoom, wipe-up — were a canvas bug, fixed 2026-09-22 by paint order and FFmpeg-matched maths; export was already correct. They are not part of this phase.)

## 2. Fixed decisions

| # | Decision | Choice |
|---|---|---|
| 1 | Runtime | **ONNX Runtime through `ort` 2.0.0-rc.10**, CPU execution provider, static binaries (`download-binaries`, the crate default). If the installer passes the 180 MB budget (§15 of the spine) the fallback is `load-dynamic` with the runtime library fetched beside the models. |
| 2 | Crate | **`photos-ml` depends only on `photos-core`** (spine §4.3). It takes pixels and samples, never paths it decodes itself: the binary decodes with `photos-media` and hands over buffers. It performs no network I/O. |
| 3 | Model delivery | **Fetched automatically** on first launch after the update (decided 2026-09-22), in the background, from a static manifest; SHA-256 verified; stored in `config_dir/JL-Studios/Lumen/models/`. A switch in Settings → Smart features turns fetching and inference off. The request is a plain HTTPS GET of a public file — no identifier, no account, no telemetry — and that is the only thing it sends. |
| 4 | Hosting | Model files are attached to the `models-v1` release of the public repository **05Narwhal/lumen-models** (the Lumen repository itself is private, and a private release cannot be downloaded without a login). The manifest (`crates/photos-models/models.json`, compiled in) names the URL, size and SHA-256 of each file, and the preprocessing each model expects. |
| 5 | Absent models | **Every algorithm degrades to today's behaviour** when its model or a photo's embedding is missing. Nothing fails, nothing waits on a download; missing ML is a state, like missing FFmpeg. |
| 6 | Where the algorithms live | The *decisions* (curation, grouping, transitions, snapping, music scoring) stay in `photos-core`, as pure functions over plain data (`Vec<i8>` embeddings, beat lists). `photos-core` never learns a model exists. `photos-ml` only turns pixels and samples into numbers. Audio DSP that both sides need — the mel spectrogram, spectral flux and the beat decoder — is one implementation in `photos-core::audio` and `photos-core::beatgrid`, so the learned front end (in `photos-ml`) and the DSP front end (called from `photos-media`) share it. |
| 7 | Determinism | Same inputs, same output. Ties are broken by id, randomness is seeded from the asset ids, as `slideshow::seeded` does today. The preview/export golden file stays authoritative. |
| 8 | Licences | Only models whose weights allow commercial redistribution. **madmom's weights are CC BY-NC-SA and are excluded.** Each model's licence is recorded in the manifest and in `docs/THIRD-PARTY-MODELS.md`. |

## 3. Models (7a)

| Role | Model | Input | Output |
|---|---|---|---|
| Image embedding | **CLIP ViT-B/32** image tower with **OpenAI's weights** (MIT) loaded through OpenCLIP (`pretrained="openai"`), exported to ONNX with fp16 weights (176 MB; dynamic int8 moved embeddings to cosine 0.970 of fp32, too far for the thresholds below). OpenAI's weights, not LAION-2B's, because the aesthetic head below was trained on that embedding space. | 224×224 RGB, CLIP mean/std | 512-d, L2-normalised |
| Aesthetic score | LAION **aesthetic predictor** linear head for ViT-B/32 (MIT) — 512 weights + bias, stored in the manifest, not an ONNX file | an embedding | a score, rescaled to 0..1 |
| Beat and downbeat activation | a small convolutional/transformer beat tracker of the *Beat This!* class (ISMIR 2024), exported to ONNX, **only if its weights' licence passes §2.8** (checked 2026-09-22: code and published weights are MIT — shipped; see `docs/THIRD-PARTY-MODELS.md`) | log-mel spectrogram at the model's frame rate | per-frame beat and downbeat probabilities |

**Prototypes** — the fixed phrases 7a compares photos against ("a photo that feels calm", "a screenshot", …) — are embedded **offline** by the export script with the same model's text tower and shipped as data (`crates/photos-ml/prototypes.json`, about 60 KB). So 7a needs no text tower and no tokenizer in Rust. 7b adds the text tower when search needs free text at query time.

Budget: the ONNX files together ≤ 250 MB on disk — measured 185.5 MB (CLIP fp16 176.2 MB, Beat This! small 9.3 MB). Inference on the dev machine: 18.8 embeddings/s in batches of 16 on 4 threads (a 34k library in about half an hour of background work), a 4-minute track's activations in < 3 s.

## 4. Data model — migration 0014

```sql
CREATE TABLE asset_embeddings (
    asset_id   INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    model      TEXT NOT NULL,              -- manifest id, e.g. "clip-vit-b32-fp16@1"
    vec        BLOB NOT NULL,              -- 512 × i8, L2-normalised then ×127
    aesthetic  REAL                        -- 0..1, NULL when the head is unavailable
);
CREATE TABLE track_analysis (
    file_name   TEXT PRIMARY KEY,          -- a name inside <library>/music, never a path
    byte_size   INTEGER NOT NULL,
    modified    INTEGER NOT NULL,          -- re-analysed when size or mtime change
    analyzer    TEXT NOT NULL,             -- "dsp@1" or the beat model's manifest id
    bpm         REAL NOT NULL,
    confidence  REAL NOT NULL,
    beats       BLOB NOT NULL,             -- u32 ms, little-endian
    downbeats   BLOB NOT NULL,             -- u32 ms, empty when unknown
    energy      REAL NOT NULL              -- onsets per second, 0.. (§7)
);
```

i8 vectors: 512 bytes a photo, 128 MB at 250k — the spine's RSS budget (§15, < 500 MB idle) forbids loading them all, and nothing needs to: 7a reads embeddings per memory or per show, a few hundred at a time. 7b's search scans them in pages.

## 5. Pipeline

- **Ingest** does not change. Embedding is backfill work on the **Maintenance lane** (spine §7), exactly like `derivatives::backfill_quality`: the 256 px thumbnail is decoded, centre-cropped to 224 and embedded, 64 photos per job, until none is missing. It runs after each import and on the six-hour tick, and after a model download completes.
- A model version bump (`model` differs) re-queues its rows — the worklist is `WHERE e.model IS NOT current`.
- Track analysis runs when a track is first used by a show (sync, automatic choice), on the Maintenance lane, and is cached in `track_analysis`.
- Events: `models://progress { file, received, total }`, `models://ready`, reusing `library://changed` when embeddings land.

## 6. Cut to the beat — a beat *list*, not a tempo

One decoder, two front ends.

1. **Onset strength** per 10 ms frame (22 050 Hz, hop 220):
   - *DSP front end* (always available): log-magnitude mel spectral flux — 1024-point FFT, 64 mel bands 30–8000 Hz, half-wave-rectified frame difference summed over bands, then a local-mean subtraction over ±0.5 s. It sees a note starting in one band while another holds, which a broadband loudness rise does not.
   - *Learned front end* (when the beat model is present): the model's beat activation, resampled to 10 ms frames; its downbeat activation is kept alongside.
2. **Tempo prior** from the onset strength's autocorrelation over 60–200 BPM, weighted by a log-Gaussian centred on 120 BPM (σ = 1 octave), with **parabolic interpolation of the peak lag**, so the tempo is not quantised to whole frames.
3. **Dynamic-programming beat tracker** (Ellis 2007): `C(t) = O(t) + max over τ ∈ [t − 2P, t − P/2] of ( C(τ) − α · (ln((t − τ)/P))² )`, α = 100, P the prior period; backtrack from the best score in the last period. The result is a list of beat times that follows tempo drift and rubato. There is no single phase to drift from.
4. **Downbeats**: from the model's downbeat activation (peaks nearest each beat, grouped into bars) when present. Otherwise the DSP guesses 4/4 by picking, among the four beat phases, the one whose beats carry the most low-band (30–150 Hz) onset energy. A guess below 1.2× the runner-up is dropped rather than trusted.
5. **Confidence** = mean onset strength *on* the beats ÷ mean onset strength overall, mapped `clamp((ratio − 1) / 1.5, 0, 1)`. `MIN_CONFIDENCE` is re-tuned on the corpus in plan 7a (the click tests and the saved track), not carried over.

**Snapping** (`photos-core::slideshow::snap_to_beat_grid`) chooses each cut as an absolute beat time from the list, so error never accumulates. For each clip, the target cut is the previous cut + the clip's wanted advance. The chosen cut is the beat nearest that target, at least `MIN_BEATS_PER_CLIP` beats on, **preferring a downbeat** when one lies within ±1 beat of the target. Durations are then rebuilt as `next_cut − cut + overlap`. The track still starts at the first beat (`start_ms`). The old `snap_to_beats(bpm, first_beat_ms)` stays as a thin wrapper that synthesises an even grid, so documents and tests that use it still work.

**Transition length** follows the music: half a beat, clamped to 250–900 ms, unless the user has set one.

## 7. Music selection

Score = `2 × mood + energy + length`, each in 0..1, with `PICK_FLOOR` unchanged.

- **Mood of the photographs**: the mean embedding of the memory's (or show's) curated photos, compared by cosine with eight mood prototypes. Each prototype is a phrase pair built from Jamendo's mood tags: *happy, calm, romantic, epic, melancholic, energetic, dreamy, playful* → "a photo that feels {mood}". A softmax at temperature 0.05 turns the eight cosines into a distribution, and that distribution replaces `mood_for_memory(kind)` in `MusicWant`. Without embeddings the kind-based list is used, as now.
- **Mood of a track**: its Jamendo tags, as `suits` reads them now, weighted by the photos' distribution instead of a fixed order.
- **Energy**: photos' energy = softmax weight on *energetic + epic + playful*; a track's energy = its `track_analysis.energy` (onsets per second above the mean, normalised by 6/s) when analysed, else its tags. The score is `1 − |photos − track|`.
- **Length** as now.

CLAP-style audio embeddings (matching the audio itself to the pictures) are **not** in 7a: an extra ~150 MB model for a judgement that tags plus energy already make reasonably. Revisit after 7b.

## 8. Memory photo selection

`memories::curate` keeps its shape — greedy maximum marginal relevance, with a veto for lookalikes — and gains three signals. Each is used only when the photo has an embedding.

- **Quality** = `(3·favourite + stars + sharp_rank + 2·aesthetic − 3·junk) / 11`, clamped to 0..1. `junk` is the largest softmax weight of the *screenshot, document or receipt, whiteboard, blurry photo, accidental photo of the ground* prototypes against a *good photograph* prototype. It is only counted above 0.5, so an ordinary photo is never penalised for faintly resembling a document.
- **Repetition** = `time × place × (0.5 + 0.5 · similar)`, where `similar = clamp((cos − 0.70) / 0.25, 0, 1)`. Time and place still multiply, as Phase 6d established; content now scales the result. Two frames of the same subject in the same minutes repeat fully, while the same minutes spent on a different subject repeat half as much.
- **Veto**: dHash within 6 bits **or** cosine ≥ 0.96 (a burst frame that moved enough to fool the hash).

The cover is the first pick, as now. The measured costs in CLAUDE.md (19 ms at 250k synthetic) are re-measured: the extra work is one 512-byte dot product per candidate per round.

## 9. Slideshow grouping and transitions

**Grouping** replaces "adjacent and the same shape" inside a moment (`MOMENT_GAP_S`, unchanged):

1. **Heroes.** In a moment of four or more, up to a quarter are shown alone — Fill for a landscape, Blur for a lone portrait — but only photographs scoring more than 0.1 above the moment's median. (As first written, "the top quarter" split every pair of portraits and, in a library without ratings, picked heroes by id.)
2. **Sets.** The rest are grouped greedily. Seed = the best remaining photo. Mates are added while the frame has room (`FrameKind::capacity`), each the candidate that maximises `0.5·similar + 0.3·shape_fit + 0.2·colour_harmony`. Anything with `similar` ≥ 0.96 (a near-twin side by side looks like a mistake) is excluded. `shape_fit` is 1 when the candidate fits the layout the set is heading for (three portraits → three-up, a panorama → Stack). `colour_harmony` = 1 − the CIE76 distance between the two dominant colours, normalised by 100.
3. **Frame.** The set's size and shapes pick the frame through the existing `choose_frame` / `frame_layout` code. Quad and Feature stay available to `LayoutStyle::Grid` only, as 6d decided.
4. Order within the moment keeps 6d's rules: best first, near-duplicates never adjacent.

Without embeddings `similar` is 0.5 for every pair, and grouping reduces to shape and colour — close to today.

**Transitions** are *scored*, not drawn at random, for the join between clip *a* and clip *b*. Each of the twenty kinds (all one-to-one onto `xfade` and drawn by the canvas) gets a score from a fixed table of rules, in `photos-core::slideshow::choose_transition`:

| Situation | Favoured | Disfavoured |
|---|---|---|
| `similar(a, b)` ≥ 0.85 — a match cut | crossfade | slides, wipes |
| a new moment starts (time gap > `MOMENT_GAP_S`) | fadeblack | slices |
| either side is a multi-photo frame | slices, vslices, corner, vertopen, horzopen (6d's `GRID_SHUFFLE`) | circle, zoom |
| the cut lands on a downbeat with energy > 0.6 | none (a hard cut), zoom, slide | crossfade |
| brightness difference > 0.4 (dominant-colour luma) | fadewhite into the bright side, fadeblack into the dark | — |
| the same kind was used on either of the last two joins | — | that kind (−1.0) |

Each rule adds +1 or −1 to a kind's score. The highest score wins, and ties are broken by the seeded stream, so `LayoutStyle::Random` becomes "Smart transitions" and stays deterministic. A transition the user picked by hand is never replaced: `Transition` gains `auto: bool` (`#[serde(default)]`, so documents saved earlier count as hand-picked and are left alone). The builder sets it, and the editor clears it when the user chooses a kind. Transitions are chosen once when the show is built, from the pictures, and again when it is cut to the beat, because only then are downbeats and energy known.

## 10. People and search (plan 7b — decided here, built later)

**As vetted (2026-09-22, plan `2026-09-22-phase-7b-people-and-search.md`):** faces are **YuNet** (MIT) + **SFace** (Apache 2.0) from OpenCV Zoo rather than SCRFD/ArcFace, whose InsightFace weights are non-commercial; clustering is **Chinese Whispers** over a cosine graph (SFace's same-person cosine 0.363) rather than HDBSCAN; the text tower is fp16 (127.5 MB) with OpenAI's tokenizer through the `tokenizers` crate. Models in all: 354 MB (budget raised to 400 MB — downloads, not the installer). The list below is the original brief.

- Face detection: a SCRFD-class detector (small variant); embeddings: an ArcFace-class 512-d model. Both must pass §2.8 — several popular face weights are research-only, and the plan's first task is that check.
- Clustering: HDBSCAN-style over face embeddings (min cluster 3), incremental on import. Naming UI in a People screen; a named person feeds memory curation (+1 quality for a known face) and search.
- Scene tags: zero-shot over a fixed label list, written through `fts.rs` into `assets_fts.keywords`.
- Natural-language search: text embedding of the query (the text tower, now run at query time) against `asset_embeddings`, brute-force cosine in pages. Target ≤ 50 ms at 250k before any ANN index.

**As built, Round 4B (2026-09-26, plan `2026-09-25-round-4b-people.md`):** clustering stays Chinese Whispers, but joins moved from the nearest centroid at 0.363 to per-person exemplars. A join needs 0.45 against one of ≤ 12 exemplars and a 0.05 margin over the runner-up. The band 0.363–0.45 is asked about (suggestions). Answers and faces moved out of a person are remembered (`face_rejections`). A CLIP check of each face crop against `face:*` prototypes rejects screens, drawings, prints and statues. Detection runs at 1536 px (from 1024). People are `listed` or `removed`; there is no hidden state. Measured on Julian's library, a copy: faces he had removed that the rule would put back fell from ~55 % to 11 %; the rescan raised faces from 2,040 to 2,784 and kept all 602 of his decisions.

**As built, Round 4C (2026-10-04, plan `2026-09-25-round-4c-pets.md`):** pets are a second detector-plus-embedder pair beside faces. D-FINE-S (COCO-only, fp32) finds cats and dogs, one reading per animal. DINOv2 ViT-S/14 (fp16) describes each from a square crop. The shipped CLIP tower checks the crop (`pet:*`: drawing and toy rejected) and settles the species when ≥ 0.95 sure (`species:*`). People's grouping runs per species with thresholds chosen on DogFaceNet (join 0.55, margin 0.10, suggest 0.45, cluster 0.60). Every threshold comes from public data, none from Julian's photos. Details and limits: `docs/STATE.md` → *Pets, Round 4C*.

## 11. Testing

- `photos-core` algorithms are pure: unit tests with hand-built embeddings (unit vectors whose cosines are chosen by construction) and hand-built beat lists.
- `photos-ml` runs real models only in `#[ignore]` tests gated on `LUMEN_MODELS_DIR`. Its unit tests cover preprocessing (crop, normalisation, mel filterbank) against values computed by hand.
- Beat tracking corpus, all `#[ignore]` tests (they need FFmpeg or audio): click tracks at 90/120/140 BPM (cuts within 15 ms, as today); a **tempo ramp** 100 → 125 BPM over 60 s (every beat within 30 ms); a drone (refused); and the saved Jamendo track (must be steady; with `LUMEN_BEAT_LABELS=<Audacity label file>` of tapped beats, F-measure ≥ 0.8 at ±70 ms).
- The preview/export golden file (`src/lib/slideshow-golden.json`) gains cases for grid-grouped and beat-snapped documents, so the canvas cannot drift from Rust.
