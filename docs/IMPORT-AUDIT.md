# Import audit — what each format keeps

Round 19 §13 (1), written 2026-11-11 from the code on `round-19-c-platform`. Julian will delete
source copies (folders, cards; the iPhone source is gone already), so the question per row is:
once the source is gone, what is in the library, what can still be read from it later, and what
is gone for good.

**How an import works today.** A source lists files (`photos-import/src/source.rs:100-132`,
`volume.rs:63`, `iphone.rs:302-339`, `apple/source.rs:94-141`, `lightroom/catalog.rs:204-228`);
each is streamed once through BLAKE3 into `.incoming` and `sync_all`ed (`ingest.rs:369-386`),
deduplicated by hash (`ingest.rs:416-419`), **read back from the disk and verified**
(`ingest.rs:425`, `verify.rs`; new in this round), read for metadata (`ingest.rs:434`,
`photos-media/src/metadata.rs:89-165`), journalled and renamed into `originals/`
(`ingest.rs:459-464`), then given a row; the whole `Metadata`, probe included, is kept in
`assets.metadata_json` (`ingest.rs:493`). Originals are copied **byte for byte**, so everything
*inside* a file (gain maps, depth, aux images, MakerNotes, embedded XMP, QuickTime keys) is kept
and can be read later. What is lost is what never enters the copy: files the listing leaves out,
and links between files.

Reading is kamadak-exif 0.6 over the container (JPEG, TIFF-based RAW, HEIF/AVIF, PNG, WebP;
`metadata.rs:124-130`, fields `metadata.rs:388-460`), rawler for RAWs EXIF misses
(`metadata.rs:152-154`), a byte scan of embedded XMP for the date and `CreatorTool` only
(`photos-media/src/xmp.rs:21,36`), the ICC profile (`metadata.rs:133-137`), a gain-map marker
scan (`hdr.rs:92-145`), and ffprobe for video (`ffmpeg.rs:66-104`, merged in
`metadata.rs:101-122`). ExifTool is not used at import. A file with no date gets its modified
time (`ingest.rs:438`).

**Fields every still gets** (when the file has them): capture date (`DateTimeOriginal` →
`DateTimeDigitized` → `DateTime`, with `OffsetTime*`; GPS time and XMP weighed in;
`metadata.rs:449-460`), GPS lat/lon/altitude (`:449-451`), orientation (`:393`), size
(`:394-395`, else decoded `:142-151`), make/model/serial/lens/lens make (`:396-400`),
exposure/aperture/ISO/focal length (`:408-411`), software, white balance, flash, metering,
program, bias (`:412-447`), colour space (`:403`, ICC). File modified/created times are stored
(`ingest.rs:486-487`) where the source has them. **Never read anywhere:** sub-seconds
(`SubSecTimeOriginal`), GPS direction/speed (`GPSImgDirection`), embedded rating, label,
keywords, title, caption (XMP `xmp:Rating`, `xmp:Label`, `dc:subject`, `dc:title`,
`dc:description`; IPTC), burst id, Live Photo content identifier.

## The table

✅ kept · ⚠ copied but not read (re-readable later) · ❌ **lost today** once the source is gone

| Row | Copied? | Read into the library | Lost today | Fix |
|---|---|---|---|---|
| **JPEG** | ✅ byte for byte (`ingest.rs:369-386`) | All still fields above; Ultra HDR gain map → `hdr` (`hdr.rs:144`) | ⚠ sub-seconds, GPS direction, embedded XMP/IPTC rating/label/keywords/title/caption (`xmp.rs` reads only date and tool) | C3: embedded XMP marks through the sidecar XMP parser; sub-seconds + GPS direction in `Metadata` v5 |
| **HEIC/HEIF** (gain map, depth, aux images) | ✅; a HEIC named `.JPG` kept as HEIC (`ingest.rs:428-433`) | Still fields (kamadak reads HEIF EXIF); gain map → `hdr` (`hdr.rs:75-76`) | ⚠ depth map, portrait matte, aux images: in the original, never used; ⚠ same unread fields as JPEG | Later: depth/matte only if a feature needs them |
| **Live Photo pairs** | ✅ both halves | Paired after the batch by **filename stem within one batch**, MOV ≤ 6 s (`photos-core/src/assets.rs:326,354-384`); Apple Photos' companion `_3` renamed to the still's stem to match (`apple/source.rs:124-137`) | ❌ the **pairing** when the still and MOV arrive in different batches (a resumed, cancelled or picked-files import): they stay two items and nothing pairs them later. ❌ **wrong pairs** when two folders in one batch share a stem (`100APPLE/IMG_0001.HEIC` and `105APPLE/IMG_0001.MOV` after the counter wraps): the join has no folder condition. ⚠ content identifier (Apple MakerNote `0x0011`, QuickTime `com.apple.quicktime.content.identifier`) never read | C3: pair by content identifier, else stem **in the same folder**, across batches |
| **ProRAW / DNG** | ✅ | TIFF EXIF via kamadak; rawler when make or date is missing (`metadata.rs:152-154`); ICC | ⚠ same unread fields | — |
| **NEF and other RAW** | ✅ (`kind.rs:19` = `photos_core::formats::RAW_EXTENSIONS`) | TIFF-based RAWs via kamadak; CR3 and others via rawler (`metadata.rs:152`) | ⚠ same unread fields | — |
| **RAW + JPEG pairs** | ✅ both, as two assets | Not linked at import; the grid hides the RAW beside a JPEG of the same stem and capture time at query time (`photos-core/src/scope.rs:32,143`); duplicates review leaves them alone (`duplicates.rs:168-169`) | nothing (the link is recomputed from both files) | — |
| **PNG / GIF / WebP / AVIF / JXL** | ✅ (`kind.rs:16,20`; JXL `kind.rs:34`) | PNG `eXIf`, WebP EXIF, AVIF (HEIF) via kamadak; XMP date in PNG via `xmp.rs`; size by decoding (`metadata.rs:142-151`, Image kind only) | ⚠ JXL: kamadak has no JXL reader, so its `Exif` box is unread (date falls back to file time; unverified on a real file). ⚠ GIF/PNG without EXIF: file time | Later: JXL `Exif` box |
| **MOV / MP4** (QuickTime keys, zone, rotation, HDR/DV, slow-mo, time-lapse, cinematic, spatial) | ✅ | ffprobe: size with rotation, duration, codec, profile, pix fmt, fps, bit rate, container, `creationdate` **with zone**, `make`/`model`, ISO 6709 GPS **with altitude**, colour transfer/primaries, Dolby Vision (`ffmpeg.rs:66-104`); → HDR kind (`hdr.rs:125-140`) | ⚠ slow-mo / time-lapse / cinematic / spatial flags not read (fps is kept in `metadata_json.probe`, so slow-mo is inferable); ⚠ content identifier (pairing above). Without FFmpeg nothing is probed (`metadata.rs:91-95`) — the *Re-read* pass must cover those rows | Later: flags from QuickTime keys; C3's *Re-read* covers rows imported without FFmpeg |
| **MTS (AVCHD)** | ✅ when listed (`kind.rs:21`, checked to be a transport stream `kind.rs:65-75`) | ffprobe size/duration/codec | ❌ **on a camera card the AVCHD stream folder is never listed**: `VolumeSource` lists only `DCIM/` (`volume.rs:62-65`), and AVCHD lives in `PRIVATE/AVCHD/BDMV/STREAM/`, Sony XAVC in `PRIVATE/M4ROOT/CLIP/`. ⚠ capture date: AVCHD keeps it in the stream's MDPM SEI, which ffprobe does not report, so it falls back to file time (unverified) | C3: list those folders on a card |
| **Bursts** | Folder/card/iPhone: ✅ each frame as an ordinary photo. Apple Photos: ❌ unpicked frames | Each frame on its own; no burst id (Apple MakerNote `BurstUUID`) | ❌ **Apple Photos: frames the user did not pick are counted, not imported and not listed** (`apple/source.rs:102-105`). ⚠ burst id unread elsewhere | C3: listed in `import_skips` (burst frame). Later: an option to bring them in; burst id → stack |
| **Panoramas** | ✅ | As any still (EXIF size) | nothing | — |
| **Sidecars `.xmp`** | ❌ **not listed, not copied** (`source.rs:117` takes only `is_media_file`) | — | ❌ rating, label, keywords, title, caption and Lightroom/Capture One/darktable develop settings of a folder import | C3: copy beside the original (`asset_sidecars`), marks through `extras::apply` (`photos-core/src/extras.rs:35`); `crs:` develop not applied (follow-up) |
| **`.aae` + `IMG_E…` render** (iPhone edits) | `.aae` ❌; `IMG_E0001.HEIC` ✅ but **as a separate, unlinked photo** (it is a media file) | The render as its own asset | ❌ the edit instructions; ❌ the link render → original | C3: `.aae` as a sidecar of its original. Later: link `IMG_E…` to its original (stack or *edited version*) |
| **`.thm`** (Canon/GoPro video thumbnail with the video's EXIF) | ❌ not copied | — | ❌ the camera's EXIF for its videos (Canon writes the shooting settings only there) | C3: copy as the video's sidecar |
| **Apple Photos library** (source) | Originals ✅; Live video `_3` and RAW alternate `_4` ✅ (`apple/source.rs:121-138`) | Favourite, albums, hidden, title, caption, keywords, date with zone, location (`apple/source.rs:182-197`) | ❌ **edited versions** (`resources/renders`) and their adjustment plists: other companions are dropped (`:124`). ❌ not-downloaded (iCloud-only) items counted (`:111-113`), ❌ referenced files (outside the library) counted (`:106-108`), trashed counted (`:98-100`) — none listed | C3: each in `import_skips` with its reason. Later: import renders as edited versions; referenced files from their own paths |
| **Lightroom Classic catalog** (source) | Originals ✅; the camera JPEG beside a RAW ✅ (`lightroom/catalog.rs:228`) | Rating, pick/reject, colour label, keywords, collections, caption, develop → recipe when Lumen has none (`catalog.rs:216-223,269-280`) | ❌ virtual copies (their settings) counted only (`:205-207`); missing originals listed in the summary (`:211-213`). ⚠ Lightroom's sidecar `.xmp` of a RAW not copied (the catalog has the same marks) | C3: virtual copies and missing files in `import_skips`. Later: virtual copies as Develop snapshots |
| **Device: iPhone** (source) | ✅ every supported file under `/DCIM` (`iphone.rs:302-339`) | Modified time; **no created time** (`iphone.rs:330`) | ❌ `.AAE` (filtered by `is_supported`, `:322`); ❌ a file whose AFC info call fails is skipped silently (`:317`) | C3: both in `import_skips` |
| **Device: card / folder** (source) | ✅ supported files; hidden entries skipped (`source.rs:109`) | Modified and created times (`source.rs:125-127`) | ❌ every non-media file and every unreadable entry is skipped **silently** (`source.rs:111-119`, a log line only); ❌ card folders outside `DCIM` (MTS row) | C3: `import_skips` for each; card folders |
| **Duplicates** | Not copied again (`ingest.rs:416-419`) | The source's extras still applied to the existing asset (`ingest.rs:270-272`) | nothing lost, but only counted (`summary.skipped`), not listed | C3: `import_skips` *duplicate of asset N* |
| **Failed copies** | ❌ (by design) | Reason in `import_failures` (`ingest.rs:258,276`); a copy that does not read back fails with `verify::MISMATCH` | nothing (the source is untouched and offered again) | done (this round) |

## Gaps, ranked

Data that cannot be re-read once the source is gone comes first.

1. **Files never listed or copied, and never named** — sidecars (`.xmp`, `.aae`, `.thm`), every
   non-media file, unreadable entries, card video outside `DCIM` (AVCHD, XAVC), Apple Photos'
   unpicked burst frames, iCloud-only, referenced and trashed items, Apple edited renders,
   Lightroom virtual copies, iPhone files whose info call failed. Only some are even counted.
2. **Sidecar content** — XMP marks and `.aae` edits are gone with the source today.
3. **Pairing** — Live Photos across batches never pair, and same-stem files in two folders of one
   batch can pair wrongly; `IMG_E…` renders are unlinked from their originals.
4. **Unread fields** (all still in the originals, so the *Re-read metadata* pass can add them):
   embedded XMP/IPTC marks, sub-seconds, GPS direction, Live Photo content identifier, burst id,
   QuickTime slow-mo/time-lapse/cinematic/spatial flags, AVCHD capture date, JXL EXIF, iPhone
   file created time, every video imported without FFmpeg.

The fix list for C3 and *Later* is under C2's hand-off in
`docs/superpowers/plans/2026-11-11-round-19-c-platform.md`.
