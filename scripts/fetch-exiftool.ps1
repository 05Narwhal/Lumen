<#
.SYNOPSIS
  Downloads ExifTool for Windows (x64) into src-tauri/binaries/ as a Tauri sidecar.

  Source: Phil Harvey's Windows package (Artistic/GPL, with Strawberry Perl in
  exiftool_files/), from SourceForge — exiftool.org serves only the current page, not the
  zip. Checked against the SHA-256 exiftool.org publishes. Re-run at any time.

.EXAMPLE
  pwsh scripts/fetch-exiftool.ps1
#>
$ErrorActionPreference = "Stop"
$version = "13.59"
$sha256 = "44b512b25af500724ba579d0a53c8fc5851628b692dd5e5d94ae4a15c2cba9ec"
$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $root "src-tauri/binaries"
$triple = "x86_64-pc-windows-msvc"
New-Item -ItemType Directory -Force $dest | Out-Null

$zip = Join-Path ([System.IO.Path]::GetTempPath()) "exiftool-$($version)_64.zip"
Write-Host "Downloading ExifTool $version..."
curl.exe -L --fail --retry 5 --retry-all-errors -o $zip "https://sourceforge.net/projects/exiftool/files/exiftool-$($version)_64.zip/download"
$actual = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLowerInvariant()
if ($actual -ne $sha256) { throw "checksum mismatch: $actual" }

$unpacked = Join-Path ([System.IO.Path]::GetTempPath()) "exiftool-$version-unpacked"
if (Test-Path $unpacked) { Remove-Item -Recurse -Force $unpacked }
Expand-Archive -Path $zip -DestinationPath $unpacked
$pkg = Get-ChildItem $unpacked -Directory | Select-Object -First 1

# The launcher finds exiftool_files/ beside itself whatever it is called, and only the
# "(-k)" in its shipped name makes it wait for a key press — so the sidecar name is safe.
Copy-Item (Join-Path $pkg.FullName "exiftool(-k).exe") (Join-Path $dest "exiftool-$triple.exe") -Force
$files = Join-Path $dest "exiftool_files"
if (Test-Path $files) { Remove-Item -Recurse -Force $files }
Copy-Item (Join-Path $pkg.FullName "exiftool_files") $files -Recurse
Write-Host "  -> $(Join-Path $dest "exiftool-$triple.exe")"
& (Join-Path $dest "exiftool-$triple.exe") -ver
