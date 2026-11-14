import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config.ts";

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: "jsdom",
      globals: false,
      setupFiles: ["./src/test/setup.ts"],
      css: false,
      include: ["src/**/*.test.{ts,tsx}", "scripts/release/**/*.test.mjs"],
      // Mantine-heavy tests take ~1 s alone but pass 5 s when the whole suite shares the
      // machine; 15 s keeps a slow run from reading as a failure.
      testTimeout: 15_000,
    },
  }),
);
