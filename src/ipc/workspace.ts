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
