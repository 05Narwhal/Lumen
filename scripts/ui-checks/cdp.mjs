// A small Chrome DevTools Protocol client for layout checks (Round 9B): what jsdom cannot
// measure (CSS layout, real timing) is measured in a real Chromium, headless Edge or the app's
// own WebView2 (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`).
// Node 22+ has WebSocket built in.

import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const EDGE = {
  win32: "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  darwin: "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
  linux: "microsoft-edge",
};

/** Starts headless Edge (or `$BROWSER`) with a debugging port; returns it and how to stop it. */
export async function launch(port = 9340) {
  const bin = process.env.BROWSER ?? EDGE[process.platform];
  const profile = mkdtempSync(join(tmpdir(), "lumen-ui-check-"));
  const child = spawn(bin, ["--headless=new", "--disable-gpu", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "about:blank"], { stdio: "ignore" });
  for (let i = 0; i < 50; i++) {
    try {
      await fetch(`http://127.0.0.1:${port}/json/version`);
      return { port, stop: () => child.kill() };
    } catch {
      await new Promise((r) => setTimeout(r, 200));
    }
  }
  child.kill();
  throw new Error(`no browser answered on ${port}`);
}

/** Connects to the first page of a browser on `port`. */
export async function connect(port) {
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r, j) => {
    ws.addEventListener("open", r);
    ws.addEventListener("error", j);
  });
  let id = 0;
  const pending = new Map();
  ws.addEventListener("message", (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) {
      pending.get(m.id)(m);
      pending.delete(m.id);
    }
  });
  const send = (method, params = {}) =>
    new Promise((r) => {
      const i = ++id;
      pending.set(i, r);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
  return {
    send,
    /** Sets the viewport. */
    size: (width, height) => send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false }),
    /** Opens `url` and waits until `selector` exists (or `timeout` ms). */
    async open(url, selector, timeout = 20000) {
      await send("Page.enable");
      await send("Page.navigate", { url });
      const until = Date.now() + timeout;
      while (Date.now() < until) {
        if (await this.eval(`!!document.querySelector(${JSON.stringify(selector)})`)) return;
        await new Promise((r) => setTimeout(r, 200));
      }
      throw new Error(`${selector} never appeared at ${url}`);
    },
    /** Evaluates an expression (awaiting a promise) and returns its value. */
    async eval(expression) {
      const res = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (res.result?.exceptionDetails) throw new Error(JSON.stringify(res.result.exceptionDetails));
      return res.result?.result?.value;
    },
    close: () => ws.close(),
  };
}
