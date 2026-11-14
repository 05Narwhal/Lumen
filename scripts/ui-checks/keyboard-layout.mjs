// Settings → Keyboard: the box ends just below its last shortcut group, at every width
// (Round 9B). jsdom has no layout, so this runs in a real browser against `yarn dev`:
//
//   yarn dev                      # or any port: BASE=http://localhost:1445
//   node scripts/ui-checks/keyboard-layout.mjs
//
// Prints each width's columns and the gap; exits 1 when the box runs on below the groups.

import { connect, launch } from "./cdp.mjs";

const BASE = process.env.BASE ?? "http://localhost:1420";
const URL = `${BASE}/settings?section=keyboard&library=1&nosplash=1&theme=glass`;
/** The group box's own padding is 16 px; anything well past it is empty box. */
const MAX_GAP = 40;

const browser = await launch();
let failed = false;
try {
  const page = await connect(browser.port);
  for (const [width, height] of [[1920, 1000], [1280, 800], [900, 800]]) {
    await page.size(width, height);
    await page.open(URL, ".keybind-list__group");
    const m = await page.eval(`(() => {
      const groups = [...document.querySelectorAll(".keybind-list__group")].map((g) => g.getBoundingClientRect());
      const box = document.querySelector(".keybind-list__groups").closest(".settings-group").getBoundingClientRect();
      return { columns: new Set(groups.map((g) => Math.round(g.left))).size, last: Math.max(...groups.map((g) => g.bottom)), box: box.bottom };
    })()`);
    const gap = Math.round(m.box - m.last);
    const ok = gap >= 0 && gap <= MAX_GAP;
    failed ||= !ok;
    console.log(`${width}×${height}: ${m.columns} column(s), box ends ${gap} px below the last group ${ok ? "ok" : "FAIL"}`);
  }
  page.close();
} finally {
  browser.stop();
}
process.exit(failed ? 1 : 0);
