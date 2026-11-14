# RAW formats

How Lumen reads RAW files, what it gets wrong, and what stands in. From the Round 17 sweep (2026-11-01, plan `docs/superpowers/plans/2026-11-01-round-17-raw.md`), `rawler` 0.8.0 and Adobe DNG Converter 18.7 on Windows.

> This product includes DNG technology under license by Adobe.

## How a RAW is read

**Develop** (`photos_media::develop`):

1. `rawler` decodes the sensor data, unless the body is on the known-wrong list (`photos_raw::known_wrong`) or the decode comes out almost entirely white (`raw::implausible`). Either counts as a refusal, like rawler's own *unsupported* and a panic.
2. On a refusal, `develop::stand_in` runs the person's own Adobe DNG Converter (`-c -p0`, below normal priority, 180 s limit, killed with Lumen) on that one photo when it is opened, and develops the DNG. DNGs are cached under app data (`cache/dng/`, 20 GB, the longest-unopened trimmed first; Settings → *RAW formats* → *Clear*).
3. Without the converter, or when its DNG is refused too: the camera's JPEG, with the note *Camera rendering, not RAW*, which links to Settings → *RAW formats*.
4. Otherwise the plain refusal, which names no maker (`develop_preview::RAW_REFUSED_MESSAGE`).

An I/O error (an unplugged disk, a locked file) is never a refusal: it goes back as itself and nothing is converted.

**Thumbnails and the viewer** (`decode.rs`): rawler's embedded preview; else, or when it is under 640 px, the largest *whole* embedded JPEG (`decode::embedded_jpeg` walks to its end marker, because a cut-off JPEG decodes grey-filled); else the sensor decode, under the same refusals. The converter is never used for thumbnails (see `LICENSING-AUDIT.md`: conversion starts only when a person opens a photo). Photos marked unreadable by an older reader are tried once more (`store::READER`).

**Extensions imported** (`photos_core::formats::RAW_EXTENSIONS`): cr2 cr3 crw nef nrw arw srf sr2 dng raf orf ori rw2 rwl pef srw 3fr fff iiq erf mos mrw kdc dcr gpr x3f. Round 17 added crw, fff, rwl, ori, gpr and x3f. Sigma's **x3f** too, since `photos-raw` contains the panic on it: rawler has a `todo!()` for it, which comes back as a refusal, and the camera's JPEG stands in. `.raw` stays out, since it names too many other things.

## The sweep

**Files:** a stratified selection of CC0 files from raw.pixls.us (each body's smallest file for the main makes; every compression class of the newest bodies of the big makes; 3 of each extension; every phone and vendor DNG), checked by SHA-256. Its 146 CC BY-NC-SA files were skipped. 946 of 953 selected files were measured: 32 makes, 861 bodies.

**Tool:** `cargo run --release -p photos-media --example raw_sweep -- <file>` prints one JSON line through the public `photos-media` API: metadata, preview (size, time, hash), sensor decode (size, time, hash of the linear pixels, or the error), and a comparison with the embedded JPEG at 64 px (stops, luma correlation, chroma distance). Each file runs in its own process with a timeout, so an abort is a result. Suspects were judged by eye on contact sheets: a silent wrong decode is worse than a refusal.

| | `main` | Round 17 |
|---|---|---|
| Sensor decodes | 876 | 822 |
| Refused | 59 | 113 (54 of them on purpose, below) |
| Process aborted | 11 (all X3F) | 0: since `photos-raw`, rawler's `todo!()` on X3F comes back as a refusal, and X3F is imported with its camera JPEG |
| Full-size preview | 651 | 884 |
| Preview under 640 px | 58 | 4 |
| No preview | 237 | 58 |

The sensor output is byte-identical on all 822 files that decode in both. Every ORF, SRW and CRW file, and unknown bodies such as the Sony A7 V and FX2, now have a thumbnail.

## Wrong decodes, refused on purpose

`raw::known_wrong` and `raw::implausible`. Found by eye or by the comparison; each was worse than a refusal.

| Bodies | What rawler does | Converter's DNG |
|---|---|---|
| Canon sRAW / mRAW (CR2 with 3 values per pixel; 15 files) | green | **right**: no longer refused as DNG |
| Canon PowerShot G9 | green | still pink: refused |
| CHDK raw dumps from Canon compacts (rawler's size-only decoder; 18 files) | green | the converter can't read them |
| Nikon D1X | non-square pixels, squashed | still squashed: refused |
| Kodak DCS Pro 14n / 14nx / SLR/c / SLR/n, EasyShare Z981 / Z990, P712 | colour casts | **right** |
| Olympus E-10 | colour cast | **right** |
| Phase One H 25 | garbled frame | **right** |
| Sony DSC-F828 | colour cast | rawler's demosaic **panics** on its four-colour DNG: refused before rawler develops it |
| Samsung Galaxy S21–S23 DNGs (8 files) | all but white (`implausible`) | **right**, except the S23+ |
| Samsung Galaxy S23+, Galaxy S10 (SM-G973U) DNGs | white / refused | garish or all but white, without tripping `implausible`: refused |

The list keys on rawler's cleaned make and model. For the bodies marked *right*, only their own format is refused (`Format::Dng` lets the converter's output through).

## What Adobe DNG Converter rescues

18.7 with `-c -p0`, run on copies of every refused or suspect file. A conversion took 2.7 s at the median, 7.8 s at most.

**Rescued** (the DNG decodes and matches the camera's rendering): Nikon HE NEFs (Z 8, Z 9, Z f, Z50_2, Z5_2, Z6_3), Nikon 1 J4 / S1, Coolpix P7700, D1H, Coolscan V ED; Sony A7 V (ILCE-7M5), FX2; GoPro GPR (HERO5–11, FUSION); Hasselblad CFV 100C, CFV-50c, L2D-20c, Lunar, X1D II 50C, X2D 100C; Leica D-LUX 6; Minolta Alpha Sweet Digital, DiMAGE 5; Panasonic DC-FZ83, DMC-GM1S; Pentax K2000; Samsung EK-GN120, GX-1L; Phase One P40+; Sigma DP1, SD14, SD15 (as DNG; X3F itself isn't imported yet); and the known-wrong bodies marked *right* above.

**Not rescued** (the camera's JPEG stands in, where there is one): CHDK dumps; Sigma Merrill and Quattro X3F; Leaf AFi-II 12; Minolta RD 175; Xiaomi Yi; Panasonic DC-FZ45; Nikon Coolscan IV ED (its DNG is refused); Canon PowerShot ELPH 180, SX20 IS, SX710 HS, A620 CRWs; and the refused DNGs above (G9, D1X, F828, Galaxy S10, S23+).

**Not tried:** Blackmagic Micro Cinema Camera DNG, Casio QV CAM, Light L16 LRI, Pentax K-70 DNG, Raspberry Pi RAW, Sinarback STI.

## Known limits

- Without the converter, files whose only route is the sensor decode and are refused have no thumbnail and are marked unreadable (CHDK dumps, the Kodak DCS Pro DCRs, sRAW files without an embedded JPEG, the H 25: 23 sweep files). None had a thumbnail on `main` either.
- Lens distortion isn't corrected (compacts show it).
- The Ricoh GXR's embedded preview comes out garbled.
- Old Sony A2xx / A3xx decode 1.5–2 stops darker than the camera's JPEG, but with correct colour.
- Adobe makes no Linux version of the converter: on Linux the camera's JPEG stands in.
- A hard crash inside rawler (a stack overflow, an abort) isn't contained, only panics are, and only once `photos-raw` is built with `panic = "unwind"` (Task 6). Until then a rawler panic ends a release build.

## Re-running

Samples and drivers live outside the repo (the sweep's scratchpad). To re-run: fetch the selection described above from raw.pixls.us, run `raw_sweep` per file in a subprocess with a timeout, and compare hashes with the previous run. On Windows the converter refuses a path containing `/` ("Failed to convert", exit code 0); pass native paths, as `stand_in::native` does.
