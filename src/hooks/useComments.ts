import { useCallback, useEffect, useState } from "react";
import {
  addComment,
  deleteComment,
  listComments,
  type Comment,
  type CommentKind,
} from "../ipc/review";

export interface AddCommentInput {
  note: string;
  anchorText?: string;
  anchorOffset?: number;
  kind: CommentKind;
}

export interface UseComments {
  comments: Comment[];
  add: (input: AddCommentInput) => Promise<void>;
  remove: (commentId: string) => Promise<void>;
}

export function useComments(taskId: string, artifactPath: string): UseComments {
  const [comments, setComments] = useState<Comment[]>([]);

  const reload = useCallback(() => {
    listComments(taskId).then(setComments);
  }, [taskId]);

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

  return { comments, add, remove };
}
