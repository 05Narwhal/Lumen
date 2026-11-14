# Changelog

Each published build of Lumen, newest first. Versions are `26.X.Y-<channel>` (`docs/RELEASING.md`). The text under a version is what Lumen shows as that update's notes and the GitHub release's text, so it is written for the person using Lumen. Lumen shows it as plain text: use plain lines and `- ` bullets, never `**bold**` or `#` headings.

## 26.2.0-alpha

Lumen 26.2.0-alpha 🧩 Plugins and themes.

This is an alpha, a development build for testing. Keep a backup of photos you can't replace.

🧩 Plugins
- Lumen can now be extended with plugins: a package (.lumenplugin) you open or drop on Lumen. Before anything is installed you see who made it, whether it is signed, and exactly what it asks to do.
- Settings → Plugins lists them. Turn one off, or remove it and undo that. Each has its own settings and a log. Nothing runs until you install it, and a plugin can't read or change your library, reach the network, or start a program.
- A plugin can add themes, Develop presets, apps for Edit in and Open with, prompts and looks for AI assistants, an importer, and tools for AI assistants.
- Plugins that run code are sandboxed and stopped if they hang or crash. Plugin tools only see the photos in Develop's filmstrip and only write drafts.
- Developer mode (Settings → Plugins) loads a plugin straight from a folder and reloads it while you write it. docs/PLUGINS.md and a plugin SDK with six examples show how.

🎨 Themes and presets
- A plugin theme appears in the theme picker after the four built-in ones, in light and dark. Lumen checks the contrast of every colour, so a theme can't make it unreadable.
- A plugin's presets get their own group in Develop's presets, read-only, with Duplicate to make one you can change. Lightroom .xmp presets work too.

📥 Google Takeout
- A first-party Takeout plugin imports a Google Photos export, from an unpacked folder or straight from the takeout-*.zip files without unpacking them. It brings albums, descriptions, places, people and dates, pairs each photo with its .json, and skips duplicates.
- 2,000 photos took 1.4 s to read on a test library.

⚠️ Known limitations
- Windows only. This build isn't code-signed yet, so Windows SmartScreen may warn the first time.
- A plugin made by JL Studios shows as such only once Lumen is built with the JL Studios key; other plugins show as made by an author or as unsigned.
- Takeout: a Live Photo whose still and movie are in different zips isn't paired.
- Plugins don't answer while Lumen is closed, and there is no plugin store yet.

📦 Installing
- New to Lumen: download Lumen-Setup-26.2.0-alpha.exe and run it.
- The file ending in _x64-setup.exe is the plain installer the updater uses. You don't need it.

## 26.1.1-alpha

Lumen 26.1.1-alpha 🔧 Repair, very large images and Open with.

This is an alpha, a development build for testing. Keep a backup of photos you can't replace.

🛠️ Repair
- Import · Repair is one page now: a health card for the whole library, what's wrong and why, and Check the library. Settings → Library no longer has the repair rows; Home's "files Lumen can't open" opens this page.
- Checking against a disk, phone or folder no longer offers to swap a file for the same file. A copy that is damaged on the source too says so.
- Recover what's readable pulls what can still be read out of a cut-short photo or video. You look at it, then Keep it (and undo that) or Discard it. The photo's info shows how much was recovered.
- Settings → Storage → Clear temporary files frees the space leftovers from repairs and recoveries use. It waits while a repair is running.

🖼️ Very large images
- An image too big to decode in one piece (a 100000 × 4000 map, for example) is no longer listed as damaged. It opens reduced, the viewer loads its full resolution in the background and says how much of it is shown, and Develop edits a reduced copy and says so.

↗️ Open with
- Edit in Photoshop (or another installed editor) with Ctrl+Shift+E, or Open with in the photo, viewer and Develop menus. Editors get a copy named like IMG_1-Edit.tif beside the original; Lightroom-style and video apps get the original itself.
- Save in the editor and the photo is updated when you come back to Lumen. A file saved as a new copy next to it is brought in too. A photo made this way carries a small badge and an Edited in tag.
- Settings → Connections → External apps shows the apps Lumen found, lets you pick defaults and add your own.
- Check the library leaves those copies alone, since the editor is meant to change them.

⚠️ Known limitations
- Windows only. This build isn't code-signed yet, so Windows SmartScreen may warn the first time.
- Copies for editors are 8-bit TIFF. A Photoshop PSD saved beside a photo is brought in but can't be opened by Lumen yet.
- Open with was only tried on Windows.

📦 Installing
- New to Lumen: download Lumen-Setup-26.1.1-alpha.exe and run it.
- The file ending in _x64-setup.exe is the plain installer the updater uses. You don't need it.

## 26.1.0-alpha

Lumen 26.1.0-alpha 🚀 The first public build.

Lumen is a photo and video library and RAW developer that lives on your computer. There's no account, no cloud and no telemetry: your photos stay in a library folder you choose, and nothing leaves your PC unless you turn it on.

This is an alpha, a development build for testing. Expect bugs, and keep a backup of photos you can't replace. Windows only for now.

✨ What's in it

📥 Import and move in
- Import from folders, phones, cameras and memory cards. Lumen copies your files into its library and never changes the originals.
- Move in from Apple Photos (when it's found on this PC) and from Lightroom Classic catalogs. Lightroom presets come along too.
- A file Lumen can't read leaves the grids and counts and is listed under Settings → Library → Files Lumen can't open, with Show in folder and Try again. Nothing is deleted.

🖼️ Browse and find
- A fast grid for libraries with tens of thousands of photos and videos, plus albums, smart albums, favourites, ratings and labels.
- Search by what's in a photo ("dog on a beach"), by tags such as Taken:, Length: or Size:, and by people and pets.
- People and pets are grouped by face, the map shows where photos were taken, and Duplicates finds near-identical shots.
- Memories and slideshows with music matched to the beat and video highlights.

🎚️ Develop
- A RAW and JPEG developer. Edits are recipes, so the original is never touched, and edits stay in Develop until you export.
- Masks with brushes and gradients, Denoise, Auto, presets and snapshots.
- RAW files Lumen can't develop yet open through Adobe DNG Converter, if you have it, or from the camera's own JPEG.
- Every export asks for a folder. Formats include JPEG, HEIC and AVIF, and slideshows export as video.

🧠 Smart features, on your PC
- Search, people, pets, Denoise and music use models that run locally. Lumen downloads them when you first need them, or during setup.
- Experimental (Settings → Smart features → Experimental): AI masks find the sky, the subject, the background and each person.
- Optional AI assistants (off by default): connect Claude Code, Codex or Claude Desktop so they can retouch the photos in Develop's filmstrip, as drafts only.
- Optional Discord status (off by default): shows "Competing in Lumen" and never anything about your photos.

🔄 Updates and Setup
- Lumen Setup installs, updates, repairs and removes Lumen. It can download the smart-feature models during the install. On removal it can also delete libraries you name, and only Lumen's own files go to the Recycle Bin.
- Lumen updates itself. Choose Alpha, Beta or Stable in Settings → About → Updates. Updates download in the background and install quietly when you close Lumen.
- Before a new version upgrades a library, Lumen keeps a safety copy of its catalogue. A library last opened by a newer Lumen is left untouched.

🎨 Feels like home
- Four themes (Gallery, Darkroom, Paper, Glass), each in light and dark.
- Ctrl+K finds every action, 1 to 8 switch pages, / searches and Space gives a quick look.
- Deleted photos wait 30 days in the Trash, and most actions can be undone for a few seconds.

⚠️ Known limitations
- Windows only. Mac and Linux builds come later.
- Windows SmartScreen may warn the first time you open Lumen Setup, because this build isn't code-signed yet. Choose More info, then Run anyway.
- Alpha builds get updates first and may break things. If something goes wrong, your originals stay safe in the library's originals folder.
- Experimental features may be slow or wrong on some photos.

📦 Installing
- New to Lumen: download Lumen-Setup-26.1.0-alpha.exe and run it.
- The file ending in _x64-setup.exe is the plain installer the updater uses. You don't need it.
- The source of the third-party programs Lumen ships with (FFmpeg, ExifTool and the RAW library) is attached to this release.
