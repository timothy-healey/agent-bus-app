# agent-bus-app

A local Tauri + React app for orchestrating multi-team Claude Code agent pipelines. Define teams, connect them into a graph (forward, revise loops, gates), run workers, review documents and approve gates inline, track rolling-window usage. Ships with a Domain-Driven Design spec → plan → implement pipeline as the bundled default; users can design their own from the canvas.

The predecessor system was a tmux + bash-supervised file-queue (see `~/agent-bus/` if you cloned that). This is the UI-first rewrite from a clean DDD model.

## Status

**Pre-implementation.** Design and planning complete; code not yet started.

- ✅ Brainstorm + spec
- ✅ DDD model (7 bounded contexts, aggregates with invariants, relationships)
- ✅ Vet pass (8 findings, all resolved)
- ✅ Plan 1 (Foundation) — 17 tasks ready for execution
- ⬜ Plan 1 implementation
- ⬜ Plans 2–7

## Source-of-truth documents

- [`PRODUCT.md`](PRODUCT.md) — register, users, scene sentence, anti-references
- [`DESIGN.md`](DESIGN.md) — visual tokens (warm dark + warm light), components, anti-patterns
- [`DOMAIN.md`](DOMAIN.md) — DDD canon: 7 bounded contexts, ubiquitous language, shared kernels
- [`docs/2026-06-22-agent-bus-app-design.md`](docs/2026-06-22-agent-bus-app-design.md) — full system design spec
- [`docs/context-map.md`](docs/context-map.md) — strategic + tactical DDD model
- [`docs/vet-agent-bus-app-spec-2026-06-22.md`](docs/vet-agent-bus-app-spec-2026-06-22.md) — vet artifact with all findings resolved
- [`docs/plans/2026-06-22-plan-1-foundation.md`](docs/plans/2026-06-22-plan-1-foundation.md) — Plan 1 implementation, 17 tasks
- [`docs/assets/designs/`](docs/assets/designs/) — 8 design screenshots
- [`docs/assets/html/`](docs/assets/html/) — 12 HTML mockups from the design session

## The seven bounded contexts

1. **Pipeline Authoring** *(supplier)* — graph definition
2. **Runtime** *(supplier)* — task lifecycle + worker pools (2 aggregates)
3. **Review** *(supplier)* — artifacts + comments + revisions
4. **Usage Telemetry** *(supplier)* — rolling window + brake
5. **Runners** *(supplier)* — anti-corruption layer to Claude (CLI + API)
6. **Workspace** *(supplier)* — projects + path resolution kernel
7. **Conversational Control** *(customer)* — god terminal; consumes the OHS of all six suppliers

Plus a deliberate second shared kernel: `agent_bus_core` (ID newtypes, cross-context enums, OHS protocol types).

## Run (once Plan 1 lands)

```bash
bun install
bun tauri dev
```

## Test (once Plan 1 lands)

```bash
cargo test --manifest-path src-tauri/Cargo.toml --workspace
bun vitest run
```

## License

MIT — see [LICENSE](LICENSE).
