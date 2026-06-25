import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export interface Project {
  id: string;
  name: string;
  root_path: string;
  target_repo: string | null;
  /** A4: extra `.claude` roots scanned for skill autocomplete (beyond global). */
  skill_sources: string[];
  active_pipeline_id: string | null;
  created_at: number;
  updated_at: number;
}

export interface CreateProjectArgs {
  name: string;
  root_path: string;
}

export async function createProject(args: CreateProjectArgs): Promise<Project> {
  return await invoke<Project>("workspace_create_project", { ...args });
}

export async function listProjects(): Promise<Project[]> {
  return await invoke<Project[]>("workspace_list_projects");
}

export async function getProject(id: string): Promise<Project> {
  return await invoke<Project>("workspace_get_project", { id });
}

export async function readArtifact(
  projectId: string,
  path: string,
): Promise<string> {
  return await invoke<string>("read_artifact", {
    project_id: projectId,
    path,
  });
}

export interface GitConfig {
  author_name: string;
  author_email: string;
}

export async function setActivePipeline(
  id: string,
  pipelineId: string | null,
): Promise<void> {
  await invoke<void>("workspace_set_active_pipeline", { id, pipeline_id: pipelineId });
}

export async function removeProject(id: string): Promise<void> {
  await invoke<void>("workspace_remove_project", { id });
}

/** Activate a project's runtime: swap the active pipeline + (re)spawn its worker
 *  loops at a fresh generation. Call after creating or selecting a project so
 *  `/inject` targets it (runtime activation is no longer boot-only). */
export async function activateProject(projectId: string): Promise<void> {
  await invoke<void>("activate_project", { project_id: projectId });
}

/** Open the native folder picker; returns the chosen absolute path, or null if
 *  cancelled. Single directory selection. Sole crossing point for the
 *  `@tauri-apps/plugin-dialog` idiom (A3) — components depend on this wrapper, not
 *  the plugin (vet F3, same ACL-seal discipline as the keychain/git seams). */
export async function pickFolder(): Promise<string | null> {
  const result = await open({ directory: true, multiple: false });
  return typeof result === "string" ? result : null;
}

/** One filesystem entry returned by `list_dir` (G7). Mirrors the Rust
 *  `DirEntry` DTO (wire-contract test locks the key set). */
export interface DirEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

/** List a directory's immediate children for the in-app FileTreePicker (G7).
 *  Tilde-expanded + dirs-first backend-side; tolerant (rejects with a string on
 *  an unreadable dir). Sole crossing point for the fs read-dir idiom — the
 *  picker depends on this wrapper, never std/fs (same ACL-seal discipline as
 *  `pickFolder`/the keychain/git seams). */
export async function listDir(path: string): Promise<DirEntry[]> {
  return await invoke<DirEntry[]>("list_dir", { path });
}

/** Set (or clear) a project's target repo (A5). Clears when null/empty. */
export async function workspaceSetTargetRepo(
  id: string,
  targetRepo: string | null,
): Promise<void> {
  await invoke<void>("workspace_set_target_repo", { id, target_repo: targetRepo });
}

/** Set a project's extra skill sources (A4). Empty list = global only. */
export async function workspaceSetSkillSources(
  id: string,
  sources: string[],
): Promise<void> {
  await invoke<void>("workspace_set_skill_sources", { id, sources });
}

export interface WorktreeEntry {
  path: string;
  head: string;
  branch: string;
  stale: boolean;
}

export async function listWorktrees(projectId: string): Promise<WorktreeEntry[]> {
  return await invoke<WorktreeEntry[]>("list_worktrees", { project_id: projectId });
}

export async function removeWorktree(projectId: string, path: string): Promise<void> {
  await invoke<void>("remove_worktree", { project_id: projectId, path });
}

export async function getGitConfig(): Promise<GitConfig> {
  return await invoke<GitConfig>("git_config_get");
}

export async function setGitConfig(authorName: string, authorEmail: string): Promise<GitConfig> {
  return await invoke<GitConfig>("git_config_set", {
    author_name: authorName,
    author_email: authorEmail,
  });
}

export async function writeProjectPipeline(
  projectId: string,
  yamlRelPath: string,
  pipelineYaml: string,
  prompts: [string, string][],
): Promise<void> {
  await invoke<void>("write_project_pipeline", {
    project_id: projectId,
    yaml_rel_path: yamlRelPath,
    pipeline_yaml: pipelineYaml,
    prompts,
  });
}
