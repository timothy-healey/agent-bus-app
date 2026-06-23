import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Role = "user" | "assistant";

export interface ToolCallRequest {
  tool_name: string;
  args: unknown;
}

export type ToolCallResult =
  | { status: "ok"; result: unknown }
  | { status: "err"; error: string };

export interface ToolCall {
  request: ToolCallRequest;
  result: ToolCallResult | null;
}

export interface Turn {
  role: Role;
  text: string;
  tool_calls: ToolCall[];
  at: number;
}

export interface Conversation {
  project_id: string;
  session_id: string;
  started_at: number;
  last_message_at: number;
  turns: Turn[];
  summary_of_prior_sessions: string | null;
  history_budget_tokens: number;
}

export async function sendMessage(input: string): Promise<Conversation> {
  return await invoke<Conversation>("send_message", { input });
}

export async function getConversation(): Promise<Conversation | null> {
  return await invoke<Conversation | null>("get_conversation");
}

/// Display-only streaming fragment for the terminal. `text` is a prose fragment;
/// `reset` marks a new model step (clears the live bubble). Authoritative turns
/// still arrive via `sendMessage`'s result; this is purely for feel.
export interface ConversationDelta {
  text: string;
  reset: boolean;
}

/// Subscribe to backend display-only streaming fragments for the terminal.
export async function onConversationDelta(
  cb: (delta: ConversationDelta) => void,
): Promise<UnlistenFn> {
  return await listen<ConversationDelta>("conversation.delta", (e) => cb(e.payload));
}
