import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

// Lumen Setup's window. Port 1477 so it can run beside the app's 1420. The tokens come from the
// app's own src/styles (alias `@app`) so the two cannot drift; nothing else is shared.
export default defineConfig({
  root: here("."),
  plugins: [react()],
  resolve: { alias: { "@app": here("../src"), "@": here("./src") } },
  css: { preprocessorOptions: { scss: { additionalData: `@use "@app/styles/mixins" as *;\n` } } },
  clearScreen: false,
  build: { outDir: "dist", emptyOutDir: true, target: "es2022" },
  server: { port: 1477, strictPort: true, host: false },
});
