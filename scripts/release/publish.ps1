# Publish a Lumen release from one window (CPM's Lumen project → *Publish release* runs this).
# It asks for what release.mjs needs, runs the dry run, and only publishes after you type the
# version. The private key and its password live only in this window's environment.
#   pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/release/publish.ps1 [26.1.0-alpha]
param([string]$Version)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '../..')
Set-Location $root

$keyFile = Join-Path $env:USERPROFILE '.lumen-release/lumen.key'
$defaultLibrary = Join-Path $env:USERPROFILE 'Pictures/Lumen Library'
$repo = '05Narwhal/lumen'

function Stop-Here([string]$why) {
    Write-Host ""
    Write-Host $why -ForegroundColor Yellow
    Write-Host "Nothing was published."
    exit 1
}

if (-not $Version) {
    $current = (Get-Content package.json -Raw | ConvertFrom-Json).version
    $answer = Read-Host "Version to publish [$current]"
    $Version = if ($answer) { $answer.Trim() } else { $current }
}
$current = (Get-Content package.json -Raw | ConvertFrom-Json).version
if ($Version -ne $current) {
    Stop-Here "package.json says $current. Run 'node scripts/release/set-version.mjs $Version', write its section in docs/CHANGELOG.md, commit, then publish."
}

if (-not (Test-Path $keyFile)) {
    Stop-Here "No signing key at $keyFile. Make it once: yarn tauri signer generate -w $keyFile (docs/RELEASING.md, 'The signing key')."
}
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $keyFile -Raw
$secret = Read-Host "Password for lumen.key" -AsSecureString
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [System.Net.NetworkCredential]::new('', $secret).Password

if (-not $env:LUMEN_RELEASE_LIBRARY) {
    $answer = Read-Host "Library to test the migration on (read only, a temp copy is migrated) [$defaultLibrary]"
    $env:LUMEN_RELEASE_LIBRARY = if ($answer) { $answer.Trim() } else { $defaultLibrary }
}

$visibility = (gh repo view $repo --json visibility --jq .visibility) 2>$null
if ($visibility -eq 'PRIVATE') {
    Write-Host ""
    Write-Host "$repo is private, so no installed Lumen can see updates." -ForegroundColor Yellow
    $answer = Read-Host "Make it public now? (yes/no)"
    if ($answer -ne 'yes') { Stop-Here "Left private." }
    gh repo edit $repo --visibility public --accept-visibility-change-consequences
}

Write-Host ""
Write-Host "Dry run: every gate, nothing built or published…" -ForegroundColor Cyan
node scripts/release/release.mjs $Version --dry-run
if ($LASTEXITCODE -ne 0) { Stop-Here "The dry run refused (the NO lines above)." }

Write-Host ""
$answer = Read-Host "Every gate holds. Type $Version to build and publish it"
if ($answer -ne $Version) { Stop-Here "Not confirmed." }

node scripts/release/release.mjs $Version
if ($LASTEXITCODE -ne 0) { Stop-Here "release.mjs stopped (above). A draft may be left on GitHub; the channels were not touched." }

Write-Host ""
Write-Host "Published Lumen ${Version}: https://github.com/$repo/releases/tag/v$Version" -ForegroundColor Green
