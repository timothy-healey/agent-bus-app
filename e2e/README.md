# Tauri E2E (WebDriver) harness — S4

End-to-end UI tests that drive the **real** Tauri app via WebDriver
(`tauri-driver` + WebdriverIO). The headline spec walks a real, LLM-free flow:
launch → open the Design Session wizard → kick off from a **Template (seed)** →
step through Teams/Prompts/Wiring/Review → **create-from-draft** → see the
Project listed and active in the topbar.

---

## ⚠️ This harness CANNOT run on macOS

`tauri-driver` is the WebDriver intermediary Tauri ships. It supports **Linux**
and **Windows** only. **macOS is unsupported**: Apple's `WKWebView` exposes no
WebDriver automation surface, and `tauri-driver` has no macOS backend.

This repo is developed on macOS, so **the suite has never been run live here** —
it is a **complete, correct scaffold** verified structurally (config + specs +
runner transpile/parse clean; fenced out of the default test/build). The first
live execution must happen on a supported platform (see `TODO(s4)` below). Both
the WebdriverIO config and `run-e2e.sh` **refuse to run on macOS** with a clear
message, so the platform limit can't be violated by accident.

| Platform | WebDriver backend | Supported? |
|---|---|---|
| Linux | `WebKitWebDriver` (`webkit2gtk-driver`) | ✅ |
| Windows | Edge WebDriver (`msedgedriver`) + WebView2 runtime | ✅ |
| macOS | — (`WKWebView` has no WebDriver) | ❌ |

---

## Prerequisites (Linux / Windows)

1. Rust toolchain + the app's normal Tauri build deps.
2. `tauri-driver`:
   ```bash
   cargo install tauri-driver --locked
   ```
3. Platform WebDriver:
   - **Linux:** `sudo apt-get install -y webkit2gtk-driver xvfb`
     (xvfb for headless CI).
   - **Windows:** install `msedgedriver` matching your installed Edge version,
     on `PATH`; ensure the WebView2 runtime is present.

## Install the harness deps (install-on-demand)

The harness has its **own** `package.json` — its deps are **not** part of the
repo's root `bun install` / `bun run build` (so the default install stays lean).
Install them only when you actually run E2E:

```bash
cd e2e
bun install        # or: npm install
```

## Run

From `e2e/`:

```bash
./run-e2e.sh
```

It builds the debug Tauri binary (`cd ../src-tauri && cargo build`), installs the
e2e deps if missing, then runs WebdriverIO.

On **headless Linux CI**, wrap it in a virtual display:

```bash
xvfb-run -a ./run-e2e.sh
```

You can also run WebdriverIO directly once the debug binary is built:

```bash
cd e2e && bun run test:e2e   # wdio run ./wdio.conf.ts
```

## What the specs cover

- `specs/smoke.e2e.ts` — the app boots and renders the empty-project state.
- `specs/create-project.e2e.ts` — the **Template (seed)** kickoff → create-from-draft
  → Project-listed flow. This drives published OHS commands only (`seed_template`,
  `create_project_from_draft`, `workspace_list_projects`) through the real UI —
  **no live `claude`**. It deliberately does **not** test the describe→Generate
  kickoff, which needs a live LLM subprocess (the LLM Chat ACL) and so can't run
  headless.

## CI guidance (GitHub Actions, Linux)

E2E runs on Linux only; the macOS runner is **excluded** from E2E.

```yaml
# .github/workflows/e2e.yml (sketch — wire up in the TODO(s4) live-run pass)
jobs:
  e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: oven-sh/setup-bun@v2
      - name: Install Tauri + WebDriver deps
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev webkit2gtk-driver xvfb \
            build-essential libssl-dev libgtk-3-dev librsvg2-dev
          cargo install tauri-driver --locked
      - name: Run E2E
        working-directory: e2e
        run: xvfb-run -a ./run-e2e.sh
```

## Exclusion from the default suite

This directory is **fenced** from the repo's default tooling:

- `bun vitest run` never collects `*.e2e.ts` (`vite.config.ts` → `test.exclude`
  has `e2e/**`; specs end in `.e2e.ts`, outside vitest's `{test,spec}` include).
- `bun run build` (`tsc && vite build`) never compiles `e2e/` (root
  `tsconfig.json` `include: ["src"]` + explicit `exclude: ["e2e"]`).
- `e2e/` has its own dep island, so the root `bun install` never pulls WebdriverIO.
- The cargo workspace is unaffected.

## TODO(s4): the first live run on a supported platform

This scaffold has not been executed against a running app. On the first Linux/
Windows run:

1. **Confirm the bundled Template (seed) buttons render** in the wizard's Basics
   step (the create-project spec asserts `templateButtons.length > 0` and fails
   loudly otherwise — it must never fall back to the live-`claude` Generate path).
2. If the text/XPath selectors prove brittle, add a `data-testid` to the template
   buttons in `src/wizard/NewProjectWizard.tsx` and to the topbar/project-list
   elements, then point `helpers/selectors.ts` at them. (Deferred to the live run
   — it's a UI change, out of scope for the scaffold.)
3. Verify the debug binary path (`src-tauri/target/debug/agent-bus-app`) matches
   what `cargo build` actually emits on that platform.
4. Wire `e2e.yml` into CI on `ubuntu-latest`.
