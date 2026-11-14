<#
.SYNOPSIS
  Downloads FFmpeg + ffprobe for Windows (x64) into src-tauri/binaries/ as Tauri sidecars.

  Source: gyan.dev "essentials" release build (GPL, includes libx264, libx265, AAC, HEIF via
  the mov demuxer). Re-run at any time to update.

.EXAMPLE
  pwsh scripts/fetch-ffmpeg.ps1
#>
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "src-tauri/binaries"
$triple = "x86_64-pc-windows-msvc"
New-Item -ItemType Directory -Force $dest | Out-Null

$zip = Join-Path ([System.IO.Path]::GetTempPath()) "ffmpeg-release-essentials.zip"
Write-Host "Downloading FFmpeg essentials build..."
curl.exe -L --fail --retry 5 --retry-all-errors -o $zip "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"

Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($zip)
try {
  foreach ($name in @("ffmpeg", "ffprobe")) {
    $entry = $archive.Entries | Where-Object { $_.FullName -like "*/bin/$name.exe" } | Select-Object -First 1
    if (-not $entry) { throw "$name.exe not found in archive" }
    $out = Join-Path $dest "$name-$triple.exe"
    [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $out, $true)
    Write-Host "  -> $out"
  }
} finally {
  $archive.Dispose()
}
& (Join-Path $dest "ffmpeg-$triple.exe") -version | Select-Object -First 1
