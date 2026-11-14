"""A contact sheet to mark by eye: one row per photo, its versions side by side, and
*better / same / worse* for each row. Marks stay in the browser (localStorage) until
*Download marks* saves them as JSON beside the sheet. A local file, never published."""

import base64
import html
import io
import json
from pathlib import Path

from PIL import Image


def _uri(img: Image.Image) -> str:
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode("ascii")


def write(path: Path | str, title: str, rows: list[dict], key: str) -> None:
    """`rows`: {"id", "images": [(label, PIL image)], "caption"}. `key` names the marks."""
    parts = []
    for r in rows:
        cells = "".join(f'<figure><img src="{_uri(img)}" alt=""><figcaption>{html.escape(label)}</figcaption></figure>' for label, img in r["images"])
        rid = html.escape(r["id"])
        buttons = "".join(f'<button data-id="{rid}" data-mark="{m}">{m}</button>' for m in ("better", "same", "worse"))
        parts.append(f'<section class="row" data-id="{rid}"><div class="images">{cells}</div><p>{html.escape(r.get("caption", ""))}</p><div class="marks">{buttons}</div></section>')
    page = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title>
<style>
:root {{ --bg: #f6f5f2; --fg: #1d1c1a; --muted: #6b6862; --line: #d9d6cf; --on: #2f6f4f; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg: #161615; --fg: #ecebe7; --muted: #9a978f; --line: #34332f; --on: #6fbf93; }} }}
body {{ margin: 0; padding: 16px; background: var(--bg); color: var(--fg); font: 14px/1.4 system-ui, sans-serif; }}
header {{ display: flex; gap: 12px; align-items: center; flex-wrap: wrap; margin-bottom: 16px; }}
.row {{ border-top: 1px solid var(--line); padding: 12px 0; }}
.images {{ display: flex; gap: 8px; flex-wrap: wrap; }}
figure {{ margin: 0; }} img {{ max-width: 100%; height: auto; display: block; }}
figcaption, p {{ color: var(--muted); font-size: 12px; }}
button {{ font: inherit; padding: 4px 10px; border: 1px solid var(--line); background: transparent; color: var(--fg); border-radius: 6px; cursor: pointer; }}
button.on {{ border-color: var(--on); color: var(--on); font-weight: 600; }}
</style></head><body>
<header><h1 style="font-size:18px;margin:0">{html.escape(title)}</h1><span id="count"></span><button id="download">Download marks</button></header>
{''.join(parts)}
<script>
const KEY = {json.dumps("lumen-marks:" + key)};
let marks = {{}};
try {{ marks = JSON.parse(localStorage.getItem(KEY) || "{{}}"); }} catch (e) {{ marks = {{}}; }}
const show = () => {{
  document.querySelectorAll("button[data-mark]").forEach(b => b.classList.toggle("on", marks[b.dataset.id] === b.dataset.mark));
  document.getElementById("count").textContent = Object.keys(marks).length + " of {len(rows)} marked";
}};
document.addEventListener("click", e => {{
  const b = e.target.closest("button[data-mark]");
  if (!b) return;
  marks[b.dataset.id] = b.dataset.mark;
  try {{ localStorage.setItem(KEY, JSON.stringify(marks)); }} catch (e) {{}}
  show();
}});
document.getElementById("download").onclick = () => {{
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([JSON.stringify(marks, null, 2)], {{ type: "application/json" }}));
  a.download = "marks.json";
  a.click();
}};
show();
</script></body></html>
"""
    Path(path).write_text(page, encoding="utf-8")
