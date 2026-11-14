#!/usr/bin/env bash
# Downloads ExifTool into src-tauri/binaries/exiftool/ (the Perl script and its lib/).
# macOS and Linux run it with the system Perl. Checked against exiftool.org's SHA-256.
# Usage: scripts/fetch-exiftool.sh
set -euo pipefail
version="13.59"
sha256="668ea3acececb7235fbd0f4900e72d5f12c9b07e5c778fd36cb1e9b5828fd65a"
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/src-tauri/binaries/exiftool"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -L --fail --retry 5 -o "$tmp/et.tar.gz" \
  "https://sourceforge.net/projects/exiftool/files/Image-ExifTool-$version.tar.gz/download"
if command -v sha256sum >/dev/null; then actual="$(sha256sum "$tmp/et.tar.gz" | cut -d' ' -f1)"
else actual="$(shasum -a 256 "$tmp/et.tar.gz" | cut -d' ' -f1)"; fi
[ "$actual" = "$sha256" ] || { echo "checksum mismatch: $actual" >&2; exit 1; }

tar -xzf "$tmp/et.tar.gz" -C "$tmp"
rm -rf "$dest"
mkdir -p "$dest"
cp "$tmp/Image-ExifTool-$version/exiftool" "$dest/exiftool"
cp -R "$tmp/Image-ExifTool-$version/lib" "$dest/lib"
chmod +x "$dest/exiftool"
"$dest/exiftool" -ver
