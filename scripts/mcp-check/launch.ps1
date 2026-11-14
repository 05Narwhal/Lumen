$s = "C:/Users/Sample/AppData/Local/Temp/claude/d340f0a98b1451a1/d2615125-ce2a-42b9-a533-ccc7335811f7/scratchpad/d1"
$env:LUMEN_APP_DATA_DIR = "$s/appdata"
$env:LUMEN_DEFAULT_LIBRARY_DIR = "$s/library"
$env:WEBVIEW2_USER_DATA_FOLDER = "$s/webview"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9479"
$env:RUST_LOG = "info"
# The checkout that holds this script (scripts/mcp-check/..), wherever the folder is.
Set-Location (Resolve-Path "$PSScriptRoot/../..")
$cfg = '{"build":{"devUrl":"http://localhost:1479","beforeDevCommand":"yarn dev --port 1479 --strictPort"}}'
$cfg | Set-Content -Encoding utf8 "$s/dev-config.json"
yarn tauri dev --config "$s/dev-config.json" *> "$s/dev.log"
