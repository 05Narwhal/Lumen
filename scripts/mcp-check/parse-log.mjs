// Turns `claude -p --output-format stream-json --verbose` output into a readable call log,
// and saves every image the assistant was shown. Usage: node parse-log.mjs <in.jsonl> <outdir>
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const [, , input, outdir] = process.argv;
mkdirSync(join(outdir, "seen"), { recursive: true });
const lines = readFileSync(input, "utf8").split(/\r?\n/).filter(Boolean);
const out = [];
const calls = new Map();
let img = 0;
let turn = 0;
for (const line of lines) {
  let ev;
  try { ev = JSON.parse(line); } catch { continue; }
  if (ev.type === "assistant") {
    turn++;
    for (const c of ev.message?.content ?? []) {
      if (c.type === "text" && c.text.trim()) out.push(`\n[${turn}] ASSISTANT: ${c.text.trim()}`);
      if (c.type === "tool_use") {
        calls.set(c.id, c.name);
        out.push(`\n[${turn}] CALL ${c.name.replace("mcp__lumen__", "")} ${JSON.stringify(c.input)}`);
      }
    }
  } else if (ev.type === "user") {
    for (const c of ev.message?.content ?? []) {
      if (c.type !== "tool_result") continue;
      const parts = Array.isArray(c.content) ? c.content : [{ type: "text", text: String(c.content) }];
      for (const p of parts) {
        if (p.type === "image") {
          const file = `seen/${String(++img).padStart(2, "0")}-${(calls.get(c.tool_use_id) ?? "x").replace("mcp__lumen__", "")}.jpg`;
          writeFileSync(join(outdir, file), Buffer.from(p.source.data, "base64"));
          out.push(`   -> IMAGE ${file}`);
        } else if (p.type === "text") {
          const t = p.text.length > 1500 ? p.text.slice(0, 1500) + "…" : p.text;
          out.push(`   -> ${c.is_error ? "ERROR " : ""}${t}`);
        }
      }
    }
  } else if (ev.type === "result") {
    out.push(`\nRESULT: turns=${ev.num_turns} ms=${ev.duration_ms} cost=$${ev.total_cost_usd?.toFixed(3)} usage=${JSON.stringify(ev.usage)}`);
  }
}
writeFileSync(join(outdir, "log.txt"), out.join("\n"));
console.log(`${out.length} lines, ${img} images -> ${outdir}`);
