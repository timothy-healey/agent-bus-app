import { invoke } from "@tauri-apps/api/core";

/** The runner-key id for the anthropic-api runner key (v1.1 scope). */
export const ANTHROPIC_API_KEY_ID = "anthropic-api";

export async function setRunnerApiKey(keyId: string, key: string): Promise<void> {
  await invoke<void>("runner_set_api_key", { key_id: keyId, key });
}

export async function getRunnerApiKeyStatus(keyId: string): Promise<boolean> {
  return await invoke<boolean>("runner_get_api_key_status", { key_id: keyId });
}

export async function clearRunnerApiKey(keyId: string): Promise<void> {
  await invoke<void>("runner_clear_api_key", { key_id: keyId });
}
