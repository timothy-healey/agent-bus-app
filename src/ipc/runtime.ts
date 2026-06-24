import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { EVENTS } from "./events";

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
  /// The Run this work-item belongs to (Runtime redesign ④b). `null` for legacy
  /// single-task pool tasks; set for bounded-buffer engine work-items. The board
  /// scopes its cards by this.
  run_id?: string | null;
  /// The work-item's stable candidate/lineage key (④b). The board labels cards by
  /// this (fallback to topic/id). `null` for legacy tasks.
  item_key?: string | null;
}

export interface BrakeState {
  on: boolean;
  reason: string | null;
}

/// One execution of a pipeline — the unit the board scopes to (Runtime redesign
/// ④a/④e). The newest not-`completed` run is the active one. Mirrors the Rust
/// `Run` serde type.
export interface Run {
  id: string;
  pipeline: string;
  project_id: string;
  generator_dry: boolean;
  completed: boolean;
  created_at: number;
}

/// One stage's bounded store as the board surfaces it (④e): live `occupancy`
/// paired with the stage's authored `capacity`. Mirrors the Rust `StoreOccupancy`.
export interface StoreOccupancy {
  stage: string;
  occupancy: number;
  capacity: number;
}

export async function injectTopic(topic: string, targetRepo?: string): Promise<Task> {
  return await invoke<Task>("inject_topic", { topic, target_repo: targetRepo ?? null });
}

/// Start a run (④e). No topic is needed — the team prompts are the work (the A6
/// insight); the optional free-text `topic` is recorded as run context for the
/// rare genuinely-reusable pipeline. Returns the created `Run`.
export async function startRun(topic?: string): Promise<Run> {
  return await invoke<Run>("start_run", { topic: topic ?? null });
}

/// Every run for a project, newest first (powers the run selector — ④e).
export async function listRuns(projectId: string): Promise<Run[]> {
  return await invoke<Run[]>("list_runs", { project_id: projectId });
}

/// Per-stage store occupancy + capacity for a run (board lane indicators — ④e).
export async function runStoreOccupancy(runId: string): Promise<StoreOccupancy[]> {
  return await invoke<StoreOccupancy[]>("run_store_occupancy", { run_id: runId });
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
  return await listen<TaskLog>(EVENTS.taskLog, (e) => cb(e.payload));
}

/// Subscribe to backend `run-changed` events (a run started/completed — ④e). The
/// payload is the affected run id; the board + run selector refetch.
export async function onRunChanged(cb: (runId: string) => void): Promise<UnlistenFn> {
  return await listen<string>(EVENTS.runChanged, (e) => cb(e.payload));
}
