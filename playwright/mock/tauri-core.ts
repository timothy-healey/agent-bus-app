/// E2E alias for `@tauri-apps/api/core` (S5). Re-exports the mock `invoke` so the
/// real `src/ipc/*.ts` wrappers dispatch into the in-memory fake backend. Only the
/// surface the app actually imports (`invoke`) is provided.
export { invoke } from "./backend";
