<#
  Trims src-tauri/target without a full rebuild: removes debug/incremental (the biggest and
  cheapest to regenerate) and build artefacts older than 14 days. Prints the size before and after.

  Uses `cargo sweep` when installed; otherwise `cargo clean -p` for each workspace crate.
  Nothing outside src-tauri/target is touched, and nothing is deleted by hand except the
  incremental folder (cargo recreates it). A full wipe is `cargo clean` in src-tauri.

  Also removes the build output of the plugin workspaces (plugins/**/target: the SDK, examples and
  test fixtures). They are small and rebuild from scratch.

  Run from anywhere:  pwsh scripts/dev/clean-build.ps1 [-Days 14]
#>
param([int]$Days = 14)

$ErrorActionPreference = 'Stop'
$srcTauri = Resolve-Path (Join-Path $PSScriptRoot '../../src-tauri')
$target = Join-Path $srcTauri 'target'

function Get-DirGB($p) {
  if (-not (Test-Path $p)) { return 0.0 }
  $sum = (Get-ChildItem $p -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
  [math]::Round(([double]($sum ?? 0)) / 1GB, 2)
}

if (Get-Process -Name cargo, rustc -ErrorAction SilentlyContinue) {
  throw 'cargo or rustc is running; wait for it to finish (this script never kills processes).'
}
if (-not (Test-Path $target)) { "No target folder at $target; nothing to do."; return }

$before = Get-DirGB $target
"target before: $before GB"

$incremental = Join-Path $target 'debug/incremental'
if (Test-Path $incremental) {
  # Cargo's own cache of half-built crates; it rebuilds it on the next build.
  Remove-Item -LiteralPath $incremental -Recurse -Force
  "removed debug/incremental"
}

$sweep = Get-Command cargo-sweep -ErrorAction SilentlyContinue
if ($sweep) {
  Push-Location $srcTauri
  try { cargo sweep --time $Days } finally { Pop-Location }
} else {
  "cargo-sweep is not installed; running 'cargo clean -p' on the workspace crates instead."
  Push-Location $srcTauri
  try {
    $names = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages.name
    foreach ($n in $names) { cargo clean -p $n 2>&1 | Out-Null }
  } finally { Pop-Location }
}

$pluginsRoot = Join-Path $PSScriptRoot ../../plugins
if (Test-Path $pluginsRoot) {
  Get-ChildItem $pluginsRoot -Directory -Recurse -Filter target -Force -ErrorAction SilentlyContinue |
    Where-Object { Test-Path (Join-Path $_.Parent.FullName Cargo.toml) } |
    ForEach-Object {
      Remove-Item -LiteralPath $_.FullName -Recurse -Force
      "removed $($_.FullName)"
    }
}

$after = Get-DirGB $target
"target after:  $after GB (was $before GB)"
