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
```

## Frontend (vitest)

```bash
bun vitest          # watch mode
bun vitest run      # single pass
bun vitest --ui     # browser UI
```

## End-to-end (manual for Plan 1)

Plan 1 does not include Tauri E2E (WebDriver) tests — deferred to a later
plan. Verify the wizard + persistence manually per Task 16 step 2.

## Coverage targets

- `agent_bus_core`: 100% (the kernel; no excuse)
- `workspace`: 90%+ on the store and api modules
- React components: every prop variant + every interaction handler
