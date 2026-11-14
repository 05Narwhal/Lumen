param([string]$Token, [string]$Name = "run1", [string]$Model = "opus", [string]$Prompt = "Edit the photos I have open in Lumen so they look good.")
$s = "C:/Users/Sample/AppData/Local/Temp/claude/d340f0a98b1451a1/d2615125-ce2a-42b9-a533-ccc7335811f7/scratchpad/d1"
$dir = "$s/$Name"
New-Item -ItemType Directory -Force $dir | Out-Null
$cfg = @{ mcpServers = @{ lumen = @{ type = "http"; url = "http://127.0.0.1:47879/mcp"; headers = @{ Authorization = "Bearer $Token" } } } } | ConvertTo-Json -Depth 5
$cfg | Set-Content -Encoding utf8 "$dir/mcp.json"
Set-Location $dir
claude -p $Prompt --model $Model --mcp-config "$dir/mcp.json" --strict-mcp-config --allowedTools "mcp__lumen__*" --output-format stream-json --verbose > "$dir/stream.jsonl" 2> "$dir/stderr.txt"
node "$s/parse-log.mjs" "$dir/stream.jsonl" $dir
