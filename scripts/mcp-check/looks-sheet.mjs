// Applies each kept `AI:` look (draft_variant use) and saves view compare:split. Usage: node looks-sheet.mjs <token> <outdir>
import { writeFileSync, mkdirSync } from "node:fs";
const [, , token, outdir] = process.argv;
mkdirSync(outdir, { recursive: true });
const url = "http://127.0.0.1:47879/mcp";
let session = null, n = 0;
async function rpc(method, params, notify = false) {
  const headers = { Authorization: `Bearer ${token}`, "Content-Type": "application/json", Accept: "application/json, text/event-stream" };
  if (session) headers["Mcp-Session-Id"] = session;
  const body = notify ? { jsonrpc: "2.0", method, params } : { jsonrpc: "2.0", id: ++n, method, params };
  const r = await fetch(url, { method: "POST", headers, body: JSON.stringify(body) });
  session = r.headers.get("mcp-session-id") ?? session;
  const text = await r.text();
  if (notify) return null;
  const json = text.startsWith("{") ? text : text.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).trim()).filter(Boolean).pop();
  return JSON.parse(json);
}
const call = (name, args) => rpc("tools/call", { name, arguments: args });
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "d1-sheet", version: "1" } });
await rpc("notifications/initialized", {}, true);
const manifest = [];
for (const id of [1, 2, 3, 4, 5]) {
  const r = await call("draft_variant", { id, name: "Natural", op: "use" });
  const names = JSON.parse(r.result.content[0].text).snapshots;
  for (const snap of names) {
    const m = /^AI: (.*) · (\d+)$/.exec(snap);
    if (!m || m[1].startsWith("Probe")) continue;
    await call("draft_variant", { id, name: m[1], op: "use" });
    const v = await call("view", { id, size: 640, compare: "split" });
    const img = v.result?.content?.find((c) => c.type === "image");
    if (!img) { console.log(id, m[1], "no image"); continue; }
    const file = `${id}-${m[1].replace(/\W+/g, "_")}.jpg`;
    writeFileSync(`${outdir}/${file}`, Buffer.from(img.data, "base64"));
    manifest.push({ id, look: m[1], score: Number(m[2]), file });
  }
}
writeFileSync(`${outdir}/manifest.json`, JSON.stringify(manifest, null, 1));
console.log(manifest.length, "renders");
