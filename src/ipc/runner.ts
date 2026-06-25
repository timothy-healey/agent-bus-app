import { invoke } from "@tauri-apps/api/core";

/// Result of a `test_model` probe (G6). `ok` = the 1-token probe succeeded;
/// `unavailable` = the runner classified a model-not-found/unavailable error;
/// `error` = any other failure (rate limit, spawn, transport). `message` is a
/// human-readable detail for the UI.
export interface ModelTestResult {
  status: "ok" | "unavailable" | "error";
  message: string;
}

/// Fire a 1-token probe against `model` to confirm it is usable at authoring
/// time (G6). Live call backend-side; in tests the runner is structural-only.
/// Sole crossing point for the probe idiom — the drawer depends on this wrapper.
export async function testModel(model: string): Promise<ModelTestResult> {
  return await invoke<ModelTestResult>("test_model", { model });
}
