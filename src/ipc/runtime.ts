import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

/// Display-only live-log fragment for one running task. `delta` is a prose
/// fragment streamed from the worker as it runs; the authoritative settled state
/// still arrives via `task.changed`. Purely for feel — accumulate per task_id.
export interface TaskLog {
  task_id: string;
  delta: string;
}

/// Subscribe to backend display-only worker log fragments (R4).
export async function onTaskLog(cb: (log: TaskLog) => void): Promise<UnlistenFn> {
  return await listen<TaskLog>("task.log", (e) => cb(e.payload));
}
