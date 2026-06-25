import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

/// E2E Vite build (S5 — local Playwright frontend E2E).
///
/// Identical to the real app EXCEPT the two Tauri IPC entry points are aliased to
/// the hand-written mock backend under `playwright/mock/`. The real `src/ipc/*.ts`
/// wrappers run unchanged against the mock, so Playwright drives the genuine React
/// app in Chromium with a deterministic, scriptable, LLM-free backend — no Rust,
/// no `tauri-driver` (which can't run on macOS — see e2e/README.md and
/// playwright/README.md). This config is consumed ONLY by Playwright's webServer;
/// the default `vite.config.ts` (and thus `bun run build`) is untouched.
const mock = (p: string) => fileURLToPath(new URL(`./playwright/mock/${p}`, import.meta.url));

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@tauri-apps/api/core": mock("tauri-core.ts"),
      "@tauri-apps/api/event": mock("tauri-event.ts"),
      // The folder picker uses the dialog plugin; the wizard never invokes it in
      // the headless flow (paths are typed), but alias it to a benign stub so the
      // import resolves without bundling the native plugin.
      "@tauri-apps/plugin-dialog": mock("tauri-dialog.ts"),
    },
  },
  clearScreen: false,
  // A dedicated port so the E2E preview never collides with the dev server (1420).
  server: { port: 1430, strictPort: true },
  preview: { port: 1430, strictPort: true },
});
