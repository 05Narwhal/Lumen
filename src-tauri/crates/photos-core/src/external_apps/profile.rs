//! The built-in table (spec §7). Every identifier was checked in Round 22 C1 against a real install
//! record on the dev PC, the vendor's own documentation or a bundle's Info.plist dump; a row or a
//! rule that could not be checked is absent (the hand-off in the lane plan lists them). A rule
//! that names a wrong file only leaves the app undetected, never launches anything else.

use super::detect::Os;
use super::AppKind::{Developer, Editor, Video};
use super::Media::{Photo, Raw, Video as Vid};
use super::Writes::{InPlace, Never, Sidecar};
use super::WinRule::{AppPath, Glob, Store, Uninstall};
use super::{AppProfile, WinRule};

const NO_WIN: &[WinRule<'static>] = &[];

pub static TABLE: &[AppProfile<'static>] = &[
    AppProfile {
        id: "photoshop",
        name: "Adobe Photoshop",
        kind: Editor,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: InPlace,
        // App Paths and the Uninstall entry both seen on the dev PC (Photoshop 2024, on X:).
        win: &[AppPath("Photoshop.exe"), Uninstall { name_starts: "Adobe Photoshop", exe: "Photoshop.exe" }],
        mac: &["com.adobe.Photoshop"],
        linux: &[],
    },
    AppProfile {
        id: "lightroom-classic",
        name: "Lightroom Classic",
        kind: Developer,
        accepts: &[Photo, Raw, Vid],
        reads_raw: true,
        writes: Never,
        win: &[
            Uninstall { name_starts: "Adobe Lightroom Classic", exe: "Adobe Lightroom Classic/Lightroom.exe" },
            Glob("%ProgramFiles%/Adobe/Adobe Lightroom Classic/Lightroom.exe"),
        ],
        mac: &["com.adobe.LightroomClassicCC7"],
        linux: &[],
    },
    AppProfile {
        id: "lightroom",
        name: "Lightroom",
        kind: Developer,
        accepts: &[Photo, Raw, Vid],
        reads_raw: true,
        writes: Never,
        win: NO_WIN,
        mac: &["com.adobe.lightroomCC"],
        linux: &[],
    },
    AppProfile {
        id: "affinity-photo",
        name: "Affinity Photo",
        kind: Editor,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: InPlace,
        win: &[Glob("%ProgramFiles%/Affinity/Photo 2/Photo.exe")],
        mac: &["com.seriflabs.affinityphoto2"],
        linux: &[],
    },
    AppProfile {
        id: "gimp",
        name: "GIMP",
        kind: Editor,
        accepts: &[Photo],
        reads_raw: false,
        writes: InPlace,
        win: &[Glob("%ProgramFiles%/GIMP */bin/gimp-*.exe"), Glob("%LocalAppData%/Programs/GIMP */bin/gimp-*.exe")],
        mac: &["org.gimp.gimp-2.10"],
        linux: &["org.gimp.GIMP", "gimp"],
    },
    AppProfile {
        id: "krita",
        name: "Krita",
        kind: Editor,
        accepts: &[Photo],
        reads_raw: false,
        writes: InPlace,
        win: &[AppPath("krita.exe")],
        mac: &["org.krita"],
        linux: &["org.kde.krita"],
    },
    AppProfile {
        id: "paintdotnet",
        name: "Paint.NET",
        kind: Editor,
        accepts: &[Photo],
        reads_raw: false,
        writes: InPlace,
        win: &[Glob("%ProgramFiles%/paint.net/paintdotnet.exe")],
        mac: &[],
        linux: &[],
    },
    AppProfile {
        id: "capture-one",
        name: "Capture One",
        kind: Developer,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: Sidecar,
        win: &[AppPath("CaptureOne.exe")],
        mac: &["com.captureone.captureone16"],
        linux: &[],
    },
    AppProfile {
        id: "dxo-photolab",
        name: "DxO PhotoLab",
        kind: Developer,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: Sidecar,
        // Forum posts and a crash report name `DxO.PhotoLab.exe` under `DxO PhotoLab <version>`.
        win: &[Glob("%ProgramFiles%/DxO/DxO PhotoLab */DxO.PhotoLab.exe")],
        mac: &[],
        linux: &[],
    },
    AppProfile {
        id: "darktable",
        name: "darktable",
        kind: Developer,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: Sidecar,
        win: &[AppPath("darktable.exe")],
        mac: &["org.darktable"],
        linux: &["org.darktable.darktable"],
    },
    AppProfile {
        id: "rawtherapee",
        name: "RawTherapee",
        kind: Developer,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: Sidecar,
        win: &[AppPath("rawtherapee.exe")],
        mac: &["com.rawtherapee.RawTherapee"],
        linux: &["com.rawtherapee.RawTherapee"],
    },
    AppProfile {
        id: "pixelmator-pro",
        name: "Pixelmator Pro",
        kind: Editor,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: InPlace,
        win: NO_WIN,
        mac: &["com.pixelmatorteam.pixelmator.x"],
        linux: &[],
    },
    AppProfile {
        id: "photomator",
        name: "Photomator",
        kind: Developer,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: Sidecar,
        win: NO_WIN,
        mac: &["com.pixelmatorteam.pixelmator.touch.x.photo"],
        linux: &[],
    },
    AppProfile {
        id: "resolve",
        name: "DaVinci Resolve",
        kind: Video,
        accepts: &[Vid],
        reads_raw: false,
        writes: Never,
        // Not in App Paths on the dev PC; its default folder holds `Resolve.exe`.
        win: &[Glob("%ProgramFiles%/Blackmagic Design/DaVinci Resolve/Resolve.exe")],
        mac: &["com.blackmagic-design.DaVinciResolve"],
        linux: &[],
    },
    AppProfile {
        id: "premiere",
        name: "Premiere Pro",
        kind: Video,
        accepts: &[Vid],
        reads_raw: false,
        writes: Never,
        // App Paths seen on the dev PC (Premiere Pro 2024, on X:); no macOS id could be checked.
        win: &[AppPath("Adobe Premiere Pro.exe")],
        mac: &[],
        linux: &[],
    },
    AppProfile {
        id: "final-cut",
        name: "Final Cut Pro",
        kind: Video,
        accepts: &[Vid],
        reads_raw: false,
        writes: Never,
        win: NO_WIN,
        mac: &["com.apple.FinalCut"],
        linux: &[],
    },
    // ArtCraft (getartcraft.com, github.com/storytold). App Paths seen on the dev PC; the Linux ids
    // are the Flatpak / icon ids the projects' own READMEs give; no project states a macOS bundle id.
    AppProfile {
        id: "photocraft",
        name: "PhotoCraft",
        kind: Editor,
        accepts: &[Photo],
        reads_raw: false, // its README lists PSD, PSB, XCF, KRA, HEIC… and no camera RAW
        writes: InPlace,
        win: &[AppPath("photocraft.exe")],
        mac: &[],
        linux: &["ai.storyteller.photocraft"],
    },
    AppProfile {
        id: "lightcraft",
        name: "LightCraft",
        kind: Developer,
        accepts: &[Photo, Raw],
        reads_raw: true, // DNG, CR2, CR3, ARW, NEF, RAF, RW2, PEF, ORF, SRW per its README
        writes: Never, // its README says nothing of sidecars, so it is given a copy
        win: &[AppPath("lightcraft.exe")],
        mac: &[],
        linux: &["ai.storyteller.lightcraft"],
    },
    AppProfile {
        id: "filmcraft",
        name: "FilmCraft",
        kind: Video,
        accepts: &[Vid],
        reads_raw: false,
        writes: Never,
        win: &[AppPath("filmcraft.exe")],
        mac: &[],
        linux: &["ai.storyteller.filmcraft"],
    },
    // Windows Store apps (Round 22 fixes). Seen on the dev PC under `HKCU\Software\Classes\Local
    // Settings\…\AppModel\Repository\Packages`: `Microsoft.Windows.Photos_<v>_x64__8wekyb3d8bbwe`
    // (app `App`), `Microsoft.Paint_<v>_x64__8wekyb3d8bbwe` (app `App`) and
    // `Microsoft.ZuneMusic_<v>_x64__8wekyb3d8bbwe` (app `Microsoft.ZuneMusic`), each with
    // `<app>\Capabilities\FileAssociations` naming a ProgID per extension (`.jpg`, `.png`, `.mp4`).
    // Photos and Paint save over what they are given, so they get a copy; Media Player only plays.
    AppProfile {
        id: "windows-photos",
        name: "Photos",
        kind: Editor,
        accepts: &[Photo],
        reads_raw: false,
        writes: InPlace,
        win: &[Store { family: "Microsoft.Windows.Photos_8wekyb3d8bbwe" }],
        mac: &[],
        linux: &[],
    },
    AppProfile {
        id: "windows-paint",
        name: "Paint",
        kind: Editor,
        accepts: &[Photo],
        reads_raw: false,
        writes: InPlace,
        win: &[Store { family: "Microsoft.Paint_8wekyb3d8bbwe" }],
        mac: &[],
        linux: &[],
    },
    AppProfile {
        id: "media-player",
        name: "Media Player",
        kind: Video,
        accepts: &[Vid],
        reads_raw: false,
        writes: Never,
        win: &[Store { family: "Microsoft.ZuneMusic_8wekyb3d8bbwe" }],
        mac: &[],
        linux: &[],
    },
    // macOS's own viewers; the bundle ids are Apple's (`com.apple.Preview`, `com.apple.QuickTimePlayerX`).
    AppProfile {
        id: "preview",
        name: "Preview",
        kind: Editor,
        accepts: &[Photo, Raw],
        reads_raw: true,
        writes: InPlace,
        win: NO_WIN,
        mac: &["com.apple.Preview"],
        linux: &[],
    },
    AppProfile {
        id: "quicktime",
        name: "QuickTime Player",
        // It can trim and save over a `.mov`, so it gets a copy (an Editor sends a video's
        // original as a copy), never the original.
        kind: Editor,
        accepts: &[Vid],
        reads_raw: false,
        writes: InPlace,
        win: NO_WIN,
        mac: &["com.apple.QuickTimePlayerX"],
        linux: &[],
    },
];

/// The built-in source: the table on one OS.
pub struct BuiltIn {
    pub(super) profiles: &'static [AppProfile<'static>],
    pub(super) os: Os,
}

impl BuiltIn {
    pub fn new(os: Os) -> Self {
        Self { profiles: TABLE, os }
    }

    pub fn current() -> Self {
        Self::new(Os::current())
    }

    #[cfg(test)]
    pub(super) fn with(profiles: &'static [AppProfile<'static>], os: Os) -> Self {
        Self { profiles, os }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_every_row_can_be_found_somewhere() {
        let mut seen = std::collections::BTreeSet::new();
        for p in TABLE {
            assert!(seen.insert(p.id), "duplicate id {}", p.id);
            assert!(!p.win.is_empty() || !p.mac.is_empty() || !p.linux.is_empty(), "{} has no OS", p.id);
            assert!(!p.accepts.is_empty(), "{} accepts nothing", p.id);
        }
    }
}
