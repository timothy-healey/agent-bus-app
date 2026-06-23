# Testing — Plan 1

## Rust (workspace)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --workspace
```

By crate:

```bash
cargo test -p agent_bus_core
cargo test -p workspace
cargo test -p pipeline
cargo test -p runners
cargo test -p runtime
```

## Frontend (vitest)

```bash
bun vitest          # watch mode
bun vitest run      # single pass
bun vitest --ui     # browser UI
```

## End-to-end (S4 — WebDriver)

A `tauri-driver` + WebdriverIO harness lives in `e2e/`. It drives the real Tauri
app (launch → kick off the wizard from a bundled Template seed → create-from-draft
→ see the Project listed). It is **opt-in and platform-gated**: `tauri-driver`
supports Linux and Windows only — **not macOS** (WKWebView exposes no WebDriver),
so it can't run on this dev machine. It is excluded from `bun vitest run` and
`bun run build` and has its own dep island. See `e2e/README.md` to run it on a
supported platform / CI.

## Coverage targets

- `agent_bus_core`: 100% (the kernel; no excuse)
- `workspace`: 90%+ on the store and api modules
- React components: every prop variant + every interaction handler
