import { invoke } from "@tauri-apps/api/core";

export interface Project {
  id: string;
  name: string;
  root_path: string;
  active_pipeline_id: string | null;
  created_at: number;
  updated_at: number;
}

export interface CreateProjectArgs {
  name: string;
  root_path: string;
}

export async function createProject(args: CreateProjectArgs): Promise<Project> {
  return await invoke<Project>("workspace_create_project", args);
}

export async function listProjects(): Promise<Project[]> {
  return await invoke<Project[]>("workspace_list_projects");
}

export async function getProject(id: string): Promise<Project> {
  return await invoke<Project>("workspace_get_project", { id });
}
