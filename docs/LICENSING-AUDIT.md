# Licensing notes for a GPL-3.0 release

**Updated:** 2026-11-14. Lumen is free software under `GPL-3.0-or-later` (`LICENSE`); `photos-raw` stays `LGPL-2.1` (rawler's licence) and the plugin SDK, WIT and examples are `MIT OR Apache-2.0`. This page replaces the 2026-10-03 audit for a commercial release, which compared every dependency against a paid product. **These are engineering notes, not legal advice.**

**Dependencies.** The rule is `scripts/licenses/compat.mjs`: `yarn run licenses` refuses any crate, npm package or sidecar whose licence is not GPL-3-compatible, and writes the notices that Settings → *Licences* shows (`src/assets/licenses/notices.json`, `scripts/licenses/extra.json` for programs, data, services and credits). Re-run it whenever dependencies change. Models are data and follow `THIRD-PARTY-MODELS.md`.

## Known risks

Jamendo and Esri used to be accepted risks here. They are not any more: each is used only as its terms allow, under *Map imagery and Jamendo* below.

| | What | Why | What to do |
|---|---|---|---|
| **Decide** | **Codec patents**: HEVC (HEIC, x265), H.264 (proxies, x264), AAC | Patent licences are separate from copyright licences and HEVC has several pools. | Prefer the OS's codecs; get advice before shipping x264 / x265 beyond the FFmpeg sidecar. |
| Risk | **Photon** (place search, komoot's public instance) and **Nominatim** (exact place names, opt-in) | Shared public instances: throttled, no availability guarantee. Nominatim: 1 request/s, identifying User-Agent, caching, attribution; names are ODbL (see below). | Self-host, or keep the bundled GeoNames atlas as the only search. |

## Map imagery and Jamendo

Decided 2026-11-14 (release simplification, phase 5). Neither provider is Lumen's customer or supplier: Lumen has no account with either, and ships no key of either. Engineering notes, not legal advice.

- **Satellite imagery: EOX Sentinel-2 cloudless** (`tiles.maps.eox.at`, layer `s2cloudless-2025_3857`, about 10 m a pixel, so the map stops at zoom 15). The layer is CC BY-NC-SA 4.0 and EOX asks for the attribution on the map: Lumen shows its text and lists the layer under Settings → *Licences*. It is free for non-commercial use, which is how Lumen is distributed (free, GPL-3.0-or-later). **A fork that sells Lumen must use other imagery.** The app asks EOX for tiles of the area in view only when *Online map detail* is on.
- **Detailed imagery: Esri World Imagery, only with a person's own key.** Esri licenses its basemaps through an ArcGIS Location Platform account and key, so Lumen asks Esri for nothing until a person pastes their own free key (Settings → Connections → Map → *Detailed satellite imagery (Esri)*). The key is stored in their settings, sent only to Esri inside tile requests and never logged; no key is built in, committed or used in tests. With the key, the person is Esri's user and Esri's terms and its *Powered by Esri* attribution apply.
- **Jamendo: each person's own client id.** Lumen has no Jamendo id built in; music search is off until a person enters their own free id from `devportal.jamendo.com`, and Settings → Music says that using it means accepting Jamendo's API terms (link included). A track whose `audiodownload_allowed` is `false` is never offered, since Lumen saves a used track beside the slideshow. Each Jamendo track links back to its Jamendo page (*Open on Jamendo*), *Music from Jamendo* is shown beside the Find results, and the credit line and licence are saved beside every export.

**Stop rule.** If EOX, Esri or Jamendo objects or blocks Lumen, the next build switches that feature off: satellite imagery → no satellite layer (or another keyless source whose licence fits); Jamendo → *Use a file…* only. Nothing in a library depends on any of them.

## What the licence obliges Lumen to ship

- **GPL / LGPL source.** The FFmpeg sidecar (BtbN build, GPL-3.0 with x264 and x265) comes with its exact source in the release (`docs/RELEASING.md`). `rawler` lives alone in `crates/photos-raw` and is loaded at run time, so a person can rebuild and replace it (LGPL-2.1 §6). ExifTool (Artistic or GPL) stays a separate program.
- **Notices** for MIT, BSD, Apache-2.0, ISC, Zlib, Unicode, OFL and CC BY: Settings → *Licences*. GeoNames (CC BY 4.0), OpenStreetMap (ODbL, "© OpenStreetMap contributors"), the IJG notice for `jpeg-encoder`, MPL-2.0 source links, fonts (OFL) and models are in `extra.json`.
- **Services:** EOX Sentinel-2 cloudless (attribution on the map, CC BY-NC-SA 4.0), OpenFreeMap (attribution "OpenFreeMap © OpenMapTiles Data from OpenStreetMap", shown by MapLibre); Discord Rich Presence is local IPC to the person's own client (check Discord's Developer Terms for the Application before the first public release).
- **Models with non-commercial weights** (CC BY-NC) live only in `lumen-models` as separate downloads, never in the installer (`THIRD-PARTY-MODELS.md`).
- **Apple:** Lumen talks to Apple's own usbmuxd service and ships no Apple code. "iPhone" names the device nominatively; no Apple logos.

## Adobe DNG Converter (Round 17)

**Read 2026-11-01**, from version 18.7, the current one when read. Engineering diligence, not legal advice.

**Where the text came from.** No licence file is installed, and the agreement isn't listed on Adobe's [licences page](https://www.adobe.com/legal/licenses-terms.html). The installer `AdobeDNGConverter_x64_18_7.exe` is an Inno Setup program, and the text it shows before installing is stored in its header. It was read from there without running the installer: 67,478 bytes, SHA-256 of the UTF-8 text without its BOM, with line ends normalised to LF: `57a8aed82bec5fa14b9b6a3ed7a3a13d5867139ec3756beabfb306f569cac79f`. It is titled *ADOBE Software License Agreement*. Being generic, it names many other products and never names the DNG Converter. The converter's own binary says: "For licensing terms and conditions, see http://www.adobe.com/products/eulas/".

**Does it forbid being launched by another program? No.** It has no clause about that. What comes closest:

- **§4.2 Use Obligations:** "Customer agrees that it will not use the Software other than as permitted by this agreement and that it will not use the Software in a manner inconsistent with its design or Documentation." Adobe's documentation is on Lumen's side here. [*Command Line Support in Adobe DNG Converter 16*](https://helpx.adobe.com/camera-raw/desktop/dng-and-file-formats/digital-negative.html), linked from Adobe's DNG page as valid for 16.0 and later, opens: "Adobe DNG Converter supports invocation from the command line, which allows it to be run by automation systems (i.e., scripting) without displaying a user interface. This allows DNG conversion to be included in automated raw processing workflows." Lumen uses the documented options (`-c`, `-p0`, `-d`).
- **§2.1.7.2** is under *Server Use*, so it covers software installed on a server for network users: "the foregoing does not permit Customer to install or access (either directly or through commands, data, or instructions) the Software: … (d) as a component of a system, workflow or service accessible by more than the Permitted Number of users; or (e) for operations not initiated by an individual user (e.g., automated server processing)." Lumen is one person's desktop program on that person's own computer, and a conversion starts only when they open that photo in Develop. It does not batch a library (a spec non-goal). **Keep it that way:** a background job that converts photos nobody opened would move towards (e), even on a desktop.
- **§4.6.1 No Transfer:** no selling or sublicensing, and no authorising copies onto another person's computer. Lumen never copies Adobe's software anywhere. **§4.7 No Service Bureau:** Lumen offers no service. **§4.3 / §4.4:** no modifying and no reverse engineering; Lumen only runs the program and reads the DNG it writes.

**The open question, for the lawyer.** The agreement is boilerplate that doesn't fit a free tool. §2.1 "applies to Customers who have purchased a license", and nobody purchases this. §2.1.2.1 says software "provided without a serial number" may be used "for demonstration, evaluation, and training purposes only", with output "used only for internal, non-commercial, and non-production purposes". Read literally against a free, serial-less utility, that would restrict the converter for everyone, in conflict with what Adobe's own page says it is for: "a free utility that converts files from more than 600 cameras to DNG". It reads as a licence type for serial-less trial copies, not a term aimed at this tool, and it would bind the person running the converter whether or not Lumen launches it. But it is the clause to ask about before the first public release.

**General Terms of Use.** [Adobe's General Terms](https://www.adobe.com/legal/terms.html) (effective 2025-10-03) don't list the converter, and it isn't clear they apply to it. If they do, §6.6 forbids accessing the Software "by any means other than the interface we provide or authorize"; the documented command line is one Adobe provides. §6.4 forbids use "on behalf of any third party" or in a hosted service; Lumen runs on the person's own behalf.

**Also found: the DNG patent licence requires a notice.** Adobe's [DNG Specification patent license](https://helpx.adobe.com/camera-raw/desktop/dng-and-file-formats/digital-negative.html) grants royalty-free rights for software that reads or writes DNG, on condition that: "Any Compliant Implementation distributed under this license must include the following notice displayed in a prominent manner within its source code and documentation: 'This product includes DNG technology under license by Adobe.'" Lumen reads DNG through `rawler`, so the notice is in Settings → *Licences* (`scripts/licenses/extra.json`), `docs/RAW-FORMATS.md` and `crates/photos-raw/README.md`. The rights can also be revoked from anyone who brings a patent action against Adobe over reading or writing DNG.

**The lines Lumen keeps** (spec, *The legal line*): Lumen never ships, bundles, downloads or installs Adobe's software. It runs only the copy the person installed, offers a link to Adobe's own page, and names Adobe only to say what it uses.

## Nominatim / ODbL (exact place names, 2026-11-08)

**Read 2026-11-08**: the [Nominatim usage policy](https://operations.osmfoundation.org/policies/nominatim/) and the [OSM copyright page](https://www.openstreetmap.org/copyright). Engineering diligence, not legal advice.

- **The service.** OpenStreetMap's public Nominatim is free with no key, limited to **1 request per second** "absolute maximum", a valid identifying User-Agent, results cached, no bulk or systematic queries, and attribution. Lumen sends one request at a time at ≥ 1.1 s apart, as `Lumen/<version> (JL Studios photo library)` (no email, no user data), only a photo's coordinates (never the photo), only while the person's *Exact place names* switch is on (off by default), and keeps every answer in `place_lookups` so a position is never asked twice. A shipped app's users would all share the public instance; the policy reserves the right to block heavy users and gives no availability guarantee. **Risk at release:** fine for "barely any users" (Julian, 2026-11-08); with many users, self-host Nominatim or use a commercial OSM geocoder (the `Geocoder` trait in `src-tauri/src/places_online.rs` takes another provider).
- **The data.** Names are OSM data under the **ODbL 1.0**. The names stored on a person's photos in their own library are a *Produced Work* (attribution, no share-alike on the library itself); the lookup cache is a derivative database that never leaves the machine (not "publicly used"), so share-alike does not bite. Attribution: *© OpenStreetMap contributors* under an online-named location in the info panel, the switch's text in Settings, and Settings → *Licences* (`Nominatim` and `OpenStreetMap data` entries). **Keep it so:** never upload or sync the cache, and keep the credit next to the names.

## Before a release

1. `yarn run licenses` passes and `src/lib/licenses.test.ts` is green.
2. The source for the GPL and LGPL parts shipped (FFmpeg, `photos-raw`) is public, or in the release (`docs/RELEASING.md`).
