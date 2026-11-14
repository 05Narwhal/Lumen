#!/usr/bin/env bash
# Downloads FFmpeg + ffprobe into src-tauri/binaries/ as Tauri sidecars.
#   Linux x64:  BtbN static GPL build (ffmpeg-master-latest-linux64-gpl.tar.xz)
#   macOS:      evermeet.cx static builds (ffmpeg.zip, ffprobe.zip); universal for Intel + Apple silicon
# Usage: scripts/fetch-ffmpeg.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/src-tauri/binaries"
mkdir -p "$dest"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)
    triple="x86_64-unknown-linux-gnu"
    curl -L --fail --retry 5 -o "$tmp/ffmpeg.tar.xz" \
      "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linux64-gpl.tar.xz"
    tar -xJf "$tmp/ffmpeg.tar.xz" -C "$tmp"
    cp "$tmp"/ffmpeg-master-latest-linux64-gpl/bin/ffmpeg "$dest/ffmpeg-$triple"
    cp "$tmp"/ffmpeg-master-latest-linux64-gpl/bin/ffprobe "$dest/ffprobe-$triple"
    ;;
  Linux-aarch64)
    triple="aarch64-unknown-linux-gnu"
    curl -L --fail --retry 5 -o "$tmp/ffmpeg.tar.xz" \
      "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-linuxarm64-gpl.tar.xz"
    tar -xJf "$tmp/ffmpeg.tar.xz" -C "$tmp"
    cp "$tmp"/ffmpeg-master-latest-linuxarm64-gpl/bin/ffmpeg "$dest/ffmpeg-$triple"
    cp "$tmp"/ffmpeg-master-latest-linuxarm64-gpl/bin/ffprobe "$dest/ffprobe-$triple"
    ;;
  Darwin-arm64) triple="aarch64-apple-darwin" ;;&
  Darwin-x86_64) triple="x86_64-apple-darwin" ;;&
  Darwin-*)
    for bin in ffmpeg ffprobe; do
      curl -L --fail --retry 5 -o "$tmp/$bin.zip" "https://evermeet.cx/ffmpeg/getrelease/$bin/zip"
      unzip -oq "$tmp/$bin.zip" -d "$tmp"
      cp "$tmp/$bin" "$dest/$bin-$triple"
    done
    ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
chmod +x "$dest"/ffmpeg-* "$dest"/ffprobe-*
"$dest/ffmpeg-$triple" -version | head -1
