import { invoke } from "@tauri-apps/api/core";

export type TaskState =
  | "queued"
  | "running"
  | "gated"
  | "revising"
  | "needs_human"
  | "done"
  | "braked";

export interface Task {
  id: string;
  project_id: string;
  pipeline: string;
  topic: string;
  target_repo: string | null;
  target_scope: string | null;
  current_stage: string;
  state: TaskState;
  attempts: number;
  parent_artifact: string | null;
  review_artifact: string | null;
  created_at: number;
  updated_at: number;
}

export interface BrakeState {
  on: boolean;
  reason: string | null;
}

export async function injectTopic(topic: string, targetRepo?: string): Promise<Task> {
  return await invoke<Task>("inject_topic", { topic, target_repo: targetRepo ?? null });
}

export async function listTasks(): Promise<Task[]> {
  return await invoke<Task[]>("list_tasks");
}

export async function approveGate(taskId: string): Promise<Task> {
  return await invoke<Task>("approve_gate", { task_id: taskId });
}

export async function reviseGate(taskId: string): Promise<Task> {
  return await invoke<Task>("revise_gate", { task_id: taskId });
}

export async function rejectGate(taskId: string): Promise<Task> {
  return await invoke<Task>("reject_gate", { task_id: taskId });
}

export async function brakeOn(reason?: string): Promise<BrakeState> {
  return await invoke<BrakeState>("brake_on", { reason: reason ?? null });
}

export async function brakeOff(): Promise<BrakeState> {
  return await invoke<BrakeState>("brake_off");
}

export async function brakeState(): Promise<BrakeState> {
  return await invoke<BrakeState>("brake_state");
}

export async function scaleTeam(teamId: string): Promise<number> {
  return await invoke<number>("scale_team", { team_id: teamId });
}
