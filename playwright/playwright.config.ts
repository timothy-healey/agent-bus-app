import { defineConfig, devices } from "@playwright/test";
import { fileURLToPath } from "node:url";

/// S5 — local Playwright frontend E2E. Runs on macOS (a plain Chromium against the
/// E2E Vite build, no `tauri-driver`). The webServer builds + serves the E2E alias
/// build (vite.config.e2e.ts), which swaps the Tauri IPC for the mock backend.
const repoRoot = fileURLToPath(new URL("..", import.meta.url));
const E2E_PORT = 1430;

export default defineConfig({
  testDir: "./specs",
  testMatch: /.*\.e2e\.ts/,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? "line" : [["list"]],
  timeout: 30_000,
  expect: { timeout: 8_000 },

  use: {
    baseURL: `http://localhost:${E2E_PORT}`,
    trace: "on-first-retry",
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],

  // Build + serve the E2E alias build with Vite preview. `vite build --mode e2e`
  // honours vite.config.e2e.ts (via --config); `vite preview` serves dist on the
  // fixed E2E port. Reuses an already-running server locally for fast reruns.
  webServer: {
    command:
      `bunx vite build --config vite.config.e2e.ts && ` +
      `bunx vite preview --config vite.config.e2e.ts --port ${E2E_PORT} --strictPort`,
    cwd: repoRoot,
    url: `http://localhost:${E2E_PORT}`,
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
    stdout: "pipe",
    stderr: "pipe",
  },
});
