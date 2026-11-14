# Julian's checks in the app (2026-11-02)

Each item was still open in CLAUDE.md / `docs/STATE.md` / the plans on 2026-11-02. Mark **Pass / Fail** and add a note; Session C reproduces every failure, fixes it with a test and writes the result here.

**Before the first launch:** the real library was at schema **v30**; `main` is **v31** (0031 adds `asset_text` and `ocr_scan`, both empty at first). Dry run on a copy (scratchpad `dbcopy/`, backup of the untouched original in `dbbackup/`): the migration applied in well under a second, no row counts changed except the two new empty tables (23,074 assets before and after). The first real launch will do the same to the real file.

| # | Round | What to do | What is right | Result |
|---|---|---|---|---|
| 1 | 8A | First launch on the real library; open the log (`logs/`); open a video | Few or no warnings; a video shows its first frame quickly (note the time) | |
| 2 | 7 | Search by a tag on each grid page (library, albums, people, places, map, trash…) | Results match the tag on every page | |
| 3 | 9B | Export a memory film | The Lumen watermark is visible and fits | |
| 4 | 10 | Face radar: refuse a suggestion, scroll the grid; dates by day and by field name; app-wide keys | Radar toasts follow the photos in view; refused faces stay refused; date searches work; keys work in every overlay | |
| 5 | 14 | Settings → Experimental: tiers, **Prism**, **Spectrum** | Rows show timing; Prism retouches in Julian's style; Spectrum runs | |
| 6 | 15 | Develop: Denoise, drag the strength slider on a real photo | No freeze; if the GPU stalls it falls back to CPU | |
| 7 | 15 | Drag the Develop toolbar and the overlays' window controls | Window moves; controls work | |
| 8 | 15 | Stuck-backend banner and **Restart Lumen** | Banner appears when stuck; restart returns to the same screen with Develop edits kept | |
| 9 | 13 | Settings → Experimental → **Glyph** on; wait for the text job; search for words seen in a photo | The photo is found; activity ring shows the job | |
| 10 | 13 | Open a photo with text, press **T** | Live Text–style overlay on the words; *Text* section lists lines | |
| 11 | 13 | Tag / smart-album rule **Has text** | Only photos with text | |
| 12 | 17 RAW | Open an **HE NEF** in Develop without Adobe DNG Converter, then with it | Without: camera JPEG with the *Camera rendering, not RAW* note; with: real RAW | |
| 13 | 17 RAW | A Canon or Sony RAW, with and without the converter | Opens (or says plainly why not) | |
| 14 | 17 RAW | Export one of those photos | Export is correct | |
| 15 | 17 Discord | Settings → *Discord status* on, with a second account watching, a game and CPM running | Shows *Competing in Lumen*, four coarse activities only, nothing about photos; the game's status is not replaced wrongly | |
| 16 | 4I | People in the real app: recluster, review groups | Few wrong faces (measured precision 96.5 %, bar 98 %) | |
| 17 | 4K T3 | Search marks for tuning | Waits for his marks | |

## Open questions for Julian

1. The 23 RAW files with only a wrong-coloured decode: show that thumbnail, or none (today none)? (`2026-11-01-round-17-raw.md`, hand-off 3)
2. Is there a real Lightroom Classic `.lrcat` to check Round 11A's catalog columns and straighten sign? None found under `C:/lumen-train/`.

## Results

### Session `finish-checks`, 2026-11-03 (code side, before the walk)

**Baseline — first launch on the real library** (`logs/lumen-20261103-165123.315.log`, 63 lines, local time 16:51:23 → 16:52:48). Every `ingest failed` warning, grouped by cause, with what the files turned out to be (read-only look at the first bytes of each original; copy of `library.db` in the scratchpad):

| Count | Log text | What the file is |
|---|---|---|
| 26 | JPEG *Illegal start bytes* | 21 HEIC and 5 QuickTime/MP4 movies saved as `IMG_*.JPG` / `.JPEG` |
| 12 | `hevc … Terminating thread … -1145393733` | 9 HEIC (some saved as `IMG_*.JPG` and already repaired to `heic` by the old render path) and 3 HEIC saved as `IMG_*.MOV` |
| 7 | *Error opening input files: Invalid data* | QuickTime movies saved as `.HEIC` (5) and `.MOV` (2): *moov atom not found* |
| 4 | *unexpected end of file* | PNG cut short (3 saved as `.jpg`, 1 `.png`) |
| 2 | PNG *Invalid PNG signature* | a HEIC and a QuickTime movie saved as `.png` |
| 1 | *Memory limit exceeded* | `biome_map_….png`, a valid 100000 × 4000 PNG (887 MB), over the decoder's limit |

Also in the log, not ingest: a *stuck / slow database call* of 25 s at `assets.rs:779` (`delete_permanently`, the start-up purge of 1,187 expired trash rows). A one-off backlog on this launch (the trash had not been emptied); not changed here, worth a glance if it repeats with an empty trash.

**The finding Julian should look at:** ffmpeg decodes **none of the 52** (checked one by one). Every HEIC/QuickTime file says *partial file*, *Invalid NAL unit size* or *moov atom not found*: the copies are cut short. 50 of them (ids 6374–7022, photos taken in November 2023, `IMG_1722` … `IMG_2031` and their *- Copy* twins) came in **one import batch on 2026-09-26 at ~09:03**, so that import copied them cut short (or the source already held cut-short files); the other two (`hairstreak32_Wu.png`, `biome_map_….png`) were imported a few hours later. Routing by container therefore changes nothing for these; the originals themselves are incomplete. Lumen never touched them (`originals/` was only read). If the phone or an old backup still has the full files, re-importing them is the only way to get those photos back.

**Fixed (TDD, each test seen failing first on the log's own message):**
1. A file is ingested as what its container says (`photos_media::content_kind`, `ingest_now`): movies saved as `.JPG`/`.HEIC`/`.PNG` go to the video path, a HEIF saved as `.MOV` to the HEIF path; the catalogue's `extension` and `media_type` follow once it decodes (`assets::set_container`). Tests `the_container_brand_says_what_a_file_is_whatever_its_name`, `a_file_is_ingested_as_what_its_container_says_not_what_its_name_says`, `a_movie_saved_as_jpg_gets_its_poster_and_is_catalogued_as_a_video` (real FFmpeg, a 2 s clip saved as `IMG_1730.JPG`).
2. FFmpeg's *Invalid data* / HEVC aborts, the cut-short PNGs and the oversized PNG are told apart from real failures (`classify_ingest_failure`): a damaged or too-large original is one `info` line, already remembered as unreadable so start-up stops retrying; anything unrecognised still warns. Tests `ingest_failures_from_the_real_log_are_told_apart` (the six texts above), `a_cut_short_png_and_a_cut_short_movie_fail_as_damaged_originals` (a PNG and a real mp4 cut in half).
3. Replay on the copy (`ingest_replay_on_a_copy`, originals only read): 52 underived → 0 read, **51 damaged, 1 too large, 0 other**, so none of the 52 warning lines can appear again. The real `library.db-wal` was 58.1 M before and after.

**Why the app exited (the run that wrote this log).** No crash found: nothing in the Windows Application or System log between 16:40 and 17:30 (no *Application Error*, no WER report, no `CrashDumps`, no critical System event; the only app entry is EdgeWebView 256 at 16:51:56, a normal start-up), and the log ends like Lumen's older logs do (it writes no line when it closes; the two earlier logs also stop at their last activity). Read as a normal close. Not reproduced, so nothing changed; if it was a crash, the `Close` path of Round 15 (ends the process after 4 s) would also leave no line.

**Suite (this branch, Windows).** `cargo test --workspace`: 57 suites, 1,587 passed, 0 failed, 112 ignored. `cargo clippy --workspace --all-targets -- -D warnings`: clean. `yarn typecheck`: clean. `yarn test`: the first run, beside `cargo test`, failed 4 tests that passed when rerun alone (117/117) — `contrast.test.ts` (accent buttons), `develop/compare.test.tsx` (menu), `viewer/gui.test.tsx` (full screen + Escape), `develop-panels/gui.test.tsx` (*Auto runs once at a time…*); the second full run on a quiet machine failed only the last of those; the third was green, **176 files / 1,164 tests**. No frontend file changed on this branch, so these are load-sensitive tests, not regressions; `develop-panels` *Auto runs once at a time…* is the one to add to the known-flaky list (CLAUDE.md names only `slideshows/gui.test.tsx` and a context-menu test).

**Remaining for Julian:** the walk (items 1–17), the two questions above, and the cut-short originals. Unchanged: OCR speed re-run, Facet upload, marks for 4K.
