// Evaluate a JS file in the dev Lumen page (target picked by URL, never by type).
// Usage: node cdp.mjs <file.js> [screenshot.png]
import { readFileSync, writeFileSync } from "node:fs";

const [, , file, shot] = process.argv;
const list = await (await fetch("http://127.0.0.1:9479/json/list")).json();
const page = list.find((t) => t.url.startsWith("http://localhost:1479"));
if (!page) throw new Error("no page on localhost:1479: " + list.map((t) => t.url).join(", "));
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let n = 0;
const send = (method, params) =>
  new Promise((resolve) => {
    const id = ++n;
    const on = (m) => {
      const d = JSON.parse(m.data);
      if (d.id === id) { ws.removeEventListener("message", on); resolve(d); }
    };
    ws.addEventListener("message", on);
    ws.send(JSON.stringify({ id, method, params }));
  });
if (file) {
  const r = await send("Runtime.evaluate", { expression: readFileSync(file, "utf8"), awaitPromise: true, returnByValue: true });
  console.log(JSON.stringify(r.result?.result?.value ?? r.result?.exceptionDetails ?? r, null, 1));
}
if (shot) {
  const r = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(shot, Buffer.from(r.result.data, "base64"));
  console.log("shot", shot);
}
ws.close();
