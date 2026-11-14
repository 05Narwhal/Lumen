# Apple-formatted disks and Apple Photos libraries — findings

*2026-09-19. Superseded the same day: the "needs an elevated helper" conclusion below is wrong for USB disks. Windows lets an unelevated process read a partition device, which was measured on a real HFS+ disk. The decision and design are in architecture spec §8.4 and the Phase 4b plan.*

The request: show macOS-formatted disks (APFS, HFS+) under Connected so photos can be imported
straight from them, and import an Apple Photos library found on such a disk with everything
Lumen's features need (EXIF, favourites, albums, Live Photos, locations).

These are two separate problems. One is **reading the filesystem**, which depends on the
operating system. The other is **understanding a `.photoslibrary`**, which works the same
everywhere once its files can be read.

## 1. Conflict with the spec

Architecture spec §2 lists *"Importing existing Apple Photos / Lightroom catalogs"* as a
non-goal, to revisit only after the §17 roadmap is finished. Building part B below means
amending §2 first, and giving the work a place in §17 (for example a "Phase 5b — Apple
sources" after Edits, or after Phase 6). Part A, reading Apple-formatted disks, is not
mentioned in the spec, but on Windows it needs an elevated helper process. That is new
architecture, so it also belongs in the spec.

## 2. Part A: reading APFS and HFS+ disks

| OS | APFS | HFS+ |
|---|---|---|
| macOS | Native. Mounts, and the existing volume watcher already sees it. **Works today** for DCIM cards; a plain folder import works for anything else. | Native. Same. |
| Linux | No in-kernel driver in mainline. Needs a raw read of `/dev/sdX` (root, or membership of the `disk` group) or the user installing apfs-fuse. | Kernel `hfsplus` driver; most desktops auto-mount it read-only. The watcher sees it. **Works today.** |
| Windows | No driver. Windows gives the partition no drive letter, so `sysinfo` never lists it. | Same: no driver, no drive letter. |

**Windows is the hard case, and it is the primary dev machine.** Reading a disk Windows can't
mount means opening `\\.\PhysicalDriveN`, and **that needs administrator rights.** Lumen runs
unelevated and should keep doing so. The only clean shape is:

- a small **elevated helper binary** (a sidecar, like FFmpeg), started through a UAC prompt
  when the user clicks the disk. It parses the filesystem and streams directory listings and
  file bytes to the app over a named pipe;
- an `AppleDiskSource: ImportSource` in `photos-import` that talks to the helper, so the
  ingest pipeline, "seen" tracking, the device page and previews all work unchanged.

Finding such disks without admin rights: the Storage Management API (`MSFT_Partition` via
WMI) reports partition GPT types without elevation. APFS is `7C3457EF-0000-11AA-AA11-00306543ECAC`
and HFS+ is `48465300-0000-11AA-AA11-00306543ECAC`. *Unverified on this machine; check in a
spike before relying on it.*

Pure-Rust readers exist, all young:

- [`apfs`](https://lib.rs/crates/apfs) 0.3.0 (Sep 2026, MIT): reads from any `Read + Seek`,
  lists directories and reads files. It does **not** handle encryption (FileVault or an
  encrypted external disk), **transparent compression** (lzvn/lzfse/zlib), snapshots, or any
  volume but the first in a container. Missing compression is the risk that matters, because
  macOS compresses some files transparently.
- [`apfs-explorer`](https://github.com/enesilhaydin/apfs-explorer): a WinFsp mount driver
  whose `apfs-core` is a git dependency only.
- HFS+: [`hfsplus`](https://crates.io/crates/hfsplus) (read-only, MIT) and
  [`hfsplus-core`](https://docs.rs/hfsplus-core/latest/hfsplus_core/), which also decodes
  decmpfs compression (zlib/LZVN/LZFSE).

Estimate: an elevated helper, two parsers behind one trait, partition discovery, and a spike
on real disks. That is a phase-sized piece of work with real risk (compression, encryption,
UAC friction), for a case (a Mac-formatted disk on a Windows PC) that users can also solve by
formatting the disk as exFAT.

## 3. Part B: importing an Apple Photos library

This works on any OS once the files are readable: on macOS, on an exFAT/NTFS disk, from a
copied `.photoslibrary` folder, or through part A.

What is in the package ([osxphotos: Understanding the Photos database](https://github.com/RhetTbull/osxphotos/wiki/Understanding-the-Photos-database),
[apple-photos-forensics](https://github.com/muxcmux/apple-photos-forensics)):

- `database/Photos.sqlite`, a Core Data store. `ZASSET` (called `ZGENERICASSET` before Photos 5)
  has one row per photo or video: `ZDIRECTORY` + `ZFILENAME` give the path under
  `originals/<first letter of UUID>/`, or a full path for a *referenced* file, plus
  `ZFAVORITE`, `ZHIDDEN`, `ZTRASHEDSTATE`, `ZDATECREATED`, `ZLATITUDE`/`ZLONGITUDE`.
  Titles and captions are in `ZADDITIONALASSETATTRIBUTES`, albums in `ZGENERICALBUM` plus a
  join table `Z_<n>ASSETS`, whose number **changes between macOS versions**.
- `originals/`: the untouched files. Their EXIF is intact, so Lumen's existing ingest
  (orientation rule, device sections, HEIC/RAW/video, HDR detection, Live Photo pairing by
  content identifier) applies to them unchanged.
- `resources/renders/`: rendered edits plus an adjustments plist. Apple's adjustments can't be
  turned into a Lumen recipe; at most the render could be imported as a second item.

What Photos.sqlite adds beyond EXIF, and where it would land in Lumen:

| Apple | Lumen |
|---|---|
| Favourite | `favorites` |
| Albums, folders | albums (flattened) |
| Hidden | an album or skipped, a choice for the user |
| Recently Deleted | skipped |
| Title / caption / keywords | FTS text (Phase 3 search) |
| Date or location changed in Photos | override the EXIF values at ingest |
| People / faces | nothing until Phase 7 |

Known traps:

- **iCloud "Optimize Mac Storage"**: many originals are missing, and only small previews
  are local. The importer must count these and tell the user, not silently import thumbnails.
- **Schema drift**: column and entity numbers differ across Photos 5 to the current macOS.
  osxphotos keeps per-version maps, and its MIT-licensed test libraries would make good golden
  fixtures.
- On macOS the system library in `~/Pictures` is protected by TCC. Reading it needs Full
  Disk Access, which the user grants in System Settings.

Estimate: a `PhotosLibrarySource: ImportSource` plus a metadata side channel into ingest
(favourite, album membership, text, date/location overrides). This is well-bounded and
testable with fixtures, and it's the part users would actually notice.

## 4. Recommendation

1. **Amend spec §2 and §17** to bring part B in as its own phase after Phase 5, with its own
   plan. It's the valuable half and it doesn't depend on the filesystem.
2. **Leave part A out for now.** macOS and Linux (HFS+) already work through the existing
   watcher. On Windows the only route is an elevated helper, which is a large, risky
   addition. Revisit it if users ask after part B ships; a one-week spike (UAC helper,
   `apfs` crate on a real compressed disk, WMI discovery) would come first.
