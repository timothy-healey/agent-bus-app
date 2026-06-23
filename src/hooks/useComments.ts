import { useCallback, useEffect, useState } from "react";
import {
  addComment,
  deleteComment,
  listComments,
  reanchorComments,
  type Comment,
  type CommentKind,
  type ReanchoredComment,
} from "../ipc/review";

export interface AddCommentInput {
  note: string;
  anchorText?: string;
  anchorOffset?: number;
  kind: CommentKind;
}

export interface UseComments {
  comments: Comment[];
  /// Comments projected onto the viewed version (B1). When `versionMarkdown`
  /// is provided, addressed comments are re-anchored; otherwise every comment
  /// is `status: "open"` at its stored offset (carry-over, back-compat).
  reanchored: ReanchoredComment[];
  add: (input: AddCommentInput) => Promise<void>;
  remove: (commentId: string) => Promise<void>;
}

export function useComments(
  taskId: string,
  artifactPath: string,
  versionMarkdown?: string,
): UseComments {
  const [comments, setComments] = useState<Comment[]>([]);
  const [reanchored, setReanchored] = useState<ReanchoredComment[]>([]);

  const reload = useCallback(() => {
    listComments(taskId).then((list) => {
      setComments(list);
      if (versionMarkdown && versionMarkdown.trim()) {
        reanchorComments(taskId, versionMarkdown).then(setReanchored);
      } else {
        // Carry-over fallback when no version body is supplied. This mirrors
        // the Rust projector's no-marker default (no marker => status "open"
        // at the stored offset; see review::reanchor::reanchor_comments) and
        // must change in lockstep with it (vet F3). The backend projector
        // remains authoritative whenever a version body is present.
        setReanchored(
          list.map((c) => ({ ...c, status: "open", effective_offset: c.anchor_offset })),
        );
      }
    });
  }, [taskId, versionMarkdown]);

  useEffect(() => {
    reload();
  }, [reload]);

  const add = useCallback(
    async (input: AddCommentInput) => {
      await addComment({
        taskId,
        artifactPath,
        note: input.note,
        anchorText: input.anchorText,
        anchorOffset: input.anchorOffset,
        kind: input.kind,
      });
      reload();
    },
    [taskId, artifactPath, reload],
  );

  const remove = useCallback(
    async (commentId: string) => {
      await deleteComment(commentId);
      reload();
    },
    [reload],
  );

  return { comments, reanchored, add, remove };
}
