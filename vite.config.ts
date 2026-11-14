import { execSync } from "node:child_process";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
/** Which commit this bundle was built from, so /dev/ui can say what code is running. */
const commit = (() => {
  try {
    return execSync("git rev-parse --short HEAD").toString().trim();
  } catch {
    return "unknown";
  }
})();

/**
 * `__BUILD_COMMIT__`, substituted where it is used. Not a `define`: `define` is part of the
 * hash Vite keys its pre-bundled dependencies on, so a commit in it threw the whole cache
 * away on every commit and each `tauri dev` re-bundled Mantine, Tabler, MapLibre and the rest
 * from scratch. A plugin's name is hashed, not what it does.
 */
function buildCommit(): Plugin {
  return {
    name: "lumen-build-commit",
    transform(code) {
      return code.includes("__BUILD_COMMIT__") ? code.replaceAll("__BUILD_COMMIT__", JSON.stringify(commit)) : undefined;
    },
  };
}

const root = fileURLToPath(new URL(".", import.meta.url));
/** Top-level entries of the project root the dev server watches; everything else is ignored. */
const watchedTop = new Set(["src", "index.html", "package.json", "vite.config.ts", "postcss.config.cjs", "tsconfig.json", "tsconfig.app.json", "tsconfig.node.json"]);

function isWatched(file: string): boolean {
  const rel = path.relative(root, file);
  if (!rel || rel.startsWith("..") || path.isAbsolute(rel)) return true;
  const top = rel.split(/[\\/]/)[0];
  return watchedTop.has(top) || top.startsWith(".env");
}

export default defineConfig({
  plugins: [react(), buildCommit()],
  // The heavy dependencies, named so the first scan bundles them all in one pass.
  optimizeDeps: {
    // The app has one entry. Left to itself Vite globs every .html under the project root as
    // a possible entry — walking the 31k files of the model-export venv and crawling stray
    // pages in dist/ and .superpowers/ — before it can bundle anything.
    entries: ["index.html"],
    include: [
      "react",
      "react-dom/client",
      "react-router",
      "@mantine/core",
      "@mantine/hooks",
      "@tabler/icons-react",
      "@tanstack/react-query",
      "@tanstack/react-virtual",
      "motion/react",
      "maplibre-gl",
      "@tauri-apps/api/core",
      "@tauri-apps/api/event",
    ],
  },
  // MapLibre starts its worker as a module (`new Worker(url, { type: "module" })`), so the
  // bundled worker must be one too.
  worker: { format: "es" },
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  css: {
    preprocessorOptions: {
      scss: { additionalData: `@use "@/styles/mixins" as *;\n` },
    },
  },
  // Vite options tailored for Tauri development
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    // Only the app's own sources are watched. On Windows the watcher opens one blocking
    // `fs.watch` per file: a deny-list missed the training venv (PyTorch's DLLs, ~5,000 files)
    // and the dev server spent minutes watching it before it could serve a single stylesheet.
    watch: { ignored: (file: string) => !isWatched(file) },
    // Vite is up a minute before the Rust build finishes; transforming the app's modules in
    // that minute means the window opens on a warm server instead of a 250-request waterfall.
    // Test helpers stay out: one importing @testing-library/react made Vite find `vitest` as a
    // browser dependency and re-bundle everything with it, minutes after the window opened.
    warmup: {
      clientFiles: ["./src/main.tsx", "./src/**/*.{ts,tsx}", "!./src/**/*.test.{ts,tsx}", "!./src/**/test-utils.{ts,tsx}", "!./src/test/**"],
    },
  },
});
