// Calls Lumen's MCP `view` (split) for each id and saves the JPEGs. Usage: node renders.mjs <token> <outdir> <ids...>
import { writeFileSync, mkdirSync } from "node:fs";
const [, , token, outdir, ...ids] = process.argv;
mkdirSync(outdir, { recursive: true });
const url = "http://127.0.0.1:47879/mcp";
let session = null;
let n = 0;
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
await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "d1-renders", version: "1" } });
await rpc("notifications/initialized", {}, true);
for (const id of ids) {
  const r = await rpc("tools/call", { name: "view", arguments: { id: Number(id), size: 768, compare: "split" } });
  const img = r.result?.content?.find((c) => c.type === "image");
  if (!img) { console.log(id, JSON.stringify(r).slice(0, 300)); continue; }
  writeFileSync(`${outdir}/split-${id}.jpg`, Buffer.from(img.data, "base64"));
  console.log(id, "ok");
}
