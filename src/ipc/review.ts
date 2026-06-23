import { invoke } from "@tauri-apps/api/core";

export type CommentKind = "inline" | "direction";
export type Verdict = "approve" | "revise" | "reject";

export interface Comment {
  id: string;
  task_id: string;
  artifact_path: string;
  anchor_text: string | null;
  anchor_offset: number | null;
  note: string;
  kind: CommentKind;
  created_at: number;
}

export interface VerdictMarker {
  task_id: string;
  verdict: Verdict;
  comment_count: number;
}

export interface AddCommentArgs {
  taskId: string;
  artifactPath: string;
  note: string;
  anchorText?: string;
  anchorOffset?: number;
  kind: CommentKind;
}

export async function addComment(args: AddCommentArgs): Promise<Comment> {
  return await invoke<Comment>("add_comment", {
    task_id: args.taskId,
    artifact_path: args.artifactPath,
    note: args.note,
    anchor_text: args.anchorText ?? null,
    anchor_offset: args.anchorOffset ?? null,
    kind: args.kind,
  });
}

export async function listComments(taskId: string): Promise<Comment[]> {
  return await invoke<Comment[]>("list_comments", { task_id: taskId });
}

export async function deleteComment(commentId: string): Promise<void> {
  await invoke("delete_comment", { comment_id: commentId });
}

export async function recordVerdict(
  taskId: string,
  verdict: Verdict,
): Promise<VerdictMarker> {
  return await invoke<VerdictMarker>("record_verdict", {
    task_id: taskId,
    verdict,
  });
}

export type CommentStatus = "open" | "addressed";

/// A stored comment projected onto the viewed artifact version (B1): the
/// `Comment` fields are flattened in, plus a derived `status` and the
/// `effective_offset` (the marker position when addressed, else the stored
/// anchor offset). Read-derived — nothing new is persisted.
export interface ReanchoredComment extends Comment {
  status: CommentStatus;
  effective_offset: number | null;
}

/// Re-anchor a task's comments onto the markdown of the currently viewed
/// artifact version using `<!-- addressed: <comment-id> -->` markers.
export async function reanchorComments(
  taskId: string,
  versionMarkdown: string,
): Promise<ReanchoredComment[]> {
  return await invoke<ReanchoredComment[]>("reanchor_comments", {
    task_id: taskId,
    version_markdown: versionMarkdown,
  });
}
