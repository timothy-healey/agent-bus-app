/// <reference types="vitest" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    // S4: the WebDriver E2E harness lives in e2e/ and is NEVER run by vitest.
    // S5: the Playwright frontend E2E suite lives in playwright/ and is likewise
    // NEVER run by vitest (its specs are *.e2e.ts, outside vitest's include, but
    // we exclude the dir explicitly too). Providing `exclude` replaces vitest's
    // default, so restate node_modules/dist.
    exclude: ["**/node_modules/**", "**/dist/**", "e2e/**", "playwright/**"],
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
});
