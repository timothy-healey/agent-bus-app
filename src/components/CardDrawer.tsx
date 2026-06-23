import { useState, type CSSProperties } from "react";
import type { Task } from "../ipc/runtime";
import { useComments } from "../hooks/useComments";
import { ArtifactView } from "./ArtifactView";
import { CommentRail } from "./CommentRail";
import { ReviseComposePanel } from "./ReviseComposePanel";
import { LineageTab } from "./LineageTab";
import { CompareView, type ComparePane } from "./CompareView";
import { Button } from "./ui/Button";

type Tab = "artifact" | "live log" | "review" | "lineage";

export interface CardDrawerProps {
  task: Task;
  artifactMarkdown: string;
  /// log text for the live-log tab (v1: settled output, may be empty).
  logText?: string;
  /// upstream writer the revise routes back to (for the panel summary).
  reviseTarget?: string;
  /// Open an upstream artifact from the lineage tab (D5). When omitted, clicking
  /// a lineage entry just switches to the artifact tab showing the current body.
  onOpenArtifact?: (path: string) => void;
  /// Two artifact bodies to compare side-by-side (B2). Set by the host once the
  /// operator has picked two versions; cleared (null) when not comparing.
  compareMarkdown?: { left: string; right: string } | null;
  /// Labels for the two compare panes (basenames of the chosen paths). Optional.
  compareLabels?: { left: string; right: string };
  /// Raise the two chosen artifact paths so the host can load + supply bodies.
  onCompare?: (pathA: string, pathB: string) => void;
  /// Leave compare mode (host clears compareMarkdown).
  onExitCompare?: () => void;
  onApprove: (taskId: string) => void;
  onRevise: (taskId: string, direction: string) => void;
  onReject: (taskId: string) => void;
}

export function CardDrawer({
  task,
  artifactMarkdown,
  logText,
  reviseTarget = "the writer",
  onOpenArtifact,
  compareMarkdown,
  compareLabels,
  onCompare,
  onExitCompare,
  onApprove,
  onRevise,
  onReject,
}: CardDrawerProps) {
  const artifactPath = task.review_artifact ?? task.parent_artifact ?? `artifacts/${task.id}.md`;
  const { comments, add, remove } = useComments(task.id, artifactPath);
  const [tab, setTab] = useState<Tab>("artifact");
  const [revising, setRevising] = useState(false);
  const [activeComment, setActiveComment] = useState<string | undefined>();

  const inlineCount = comments.filter((c) => c.kind === "inline").length;
  const gated = task.state === "gated";

  const head: CSSProperties = { padding: "14px 18px", borderBottom: "1px solid var(--border)" };
  const tabBar: CSSProperties = {
    display: "flex",
    borderBottom: "1px solid var(--border)",
    background: "var(--bg-2)",
  };
  function tabStyle(t: Tab): CSSProperties {
    const active = t === tab;
    return {
      padding: "9px 16px",
      fontSize: 11.5,
      color: active ? "var(--accent)" : "var(--text-3)",
      background: active ? "var(--surface)" : "transparent",
      border: "none",
      borderRight: "1px solid var(--border)",
      cursor: "pointer",
      fontFamily: "inherit",
    };
  }
  const bodyWrap: CSSProperties = { flex: 1, display: "flex", minHeight: 0 };
  const actionBar: CSSProperties = {
    borderTop: "1px solid var(--border)",
    background: "var(--bg-2)",
    padding: "12px 16px",
    display: "flex",
    gap: 8,
    justifyContent: "flex-end",
    alignItems: "center",
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={head}>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
          <span style={{ color: "var(--accent)", fontSize: 12 }}>{task.id}</span>
          <span style={{ color: "var(--text-3)", fontSize: 11 }}>
            {task.current_stage} · a{task.attempts}
          </span>
        </div>
        <div style={{ fontSize: 14.5, color: "var(--text)" }}>{task.topic}</div>
      </div>

      <div style={tabBar} role="tablist">
        <button role="tab" aria-selected={tab === "artifact"} style={tabStyle("artifact")} onClick={() => setTab("artifact")}>
          artifact{inlineCount > 0 ? ` (${inlineCount})` : ""}
        </button>
        <button role="tab" aria-selected={tab === "live log"} style={tabStyle("live log")} onClick={() => setTab("live log")}>
          live log
        </button>
        <button role="tab" aria-selected={tab === "review"} style={tabStyle("review")} onClick={() => setTab("review")}>
          review
        </button>
        <button role="tab" aria-selected={tab === "lineage"} style={tabStyle("lineage")} onClick={() => setTab("lineage")}>
          lineage
        </button>
      </div>

      <div style={bodyWrap}>
        {tab === "artifact" && (
          <>
            <div style={{ flex: 1, minWidth: 0 }}>
              <ArtifactView
                markdown={artifactMarkdown}
                onAddComment={(note, quote, offset) =>
                  add({ note, anchorText: quote, anchorOffset: offset, kind: "inline" })
                }
              />
            </div>
            <div style={{ width: 230, flexShrink: 0 }}>
              <CommentRail
                comments={comments}
                activeId={activeComment}
                onSelect={setActiveComment}
                onDelete={remove}
              />
            </div>
          </>
        )}
        {tab === "live log" && (
          <pre
            style={{
              flex: 1,
              margin: 0,
              padding: "14px 18px",
              fontFamily: "'Berkeley Mono','JetBrains Mono',ui-monospace,monospace",
              fontSize: 11,
              color: "var(--text-3)",
              overflowY: "auto",
              whiteSpace: "pre-wrap",
            }}
          >
            {logText && logText.trim() ? logText : "no log captured for this task yet."}
          </pre>
        )}
        {tab === "review" && (
          <div style={{ flex: 1, padding: "14px 18px", color: "var(--text-2)", fontSize: 12 }}>
            {task.review_artifact
              ? `review artifact: ${task.review_artifact}`
              : "no review artifact yet. gate actions are in the bar below."}
          </div>
        )}
        {tab === "lineage" && (
          <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
            <LineageTab
              task={task}
              onOpenArtifact={(path) => {
                onExitCompare?.();
                onOpenArtifact?.(path);
                setTab("artifact");
              }}
              onCompare={onCompare}
            />
            {compareMarkdown && (
              <CompareView
                left={{ label: compareLabels?.left ?? "version A", markdown: compareMarkdown.left } as ComparePane}
                right={{ label: compareLabels?.right ?? "version B", markdown: compareMarkdown.right } as ComparePane}
              />
            )}
          </div>
        )}
      </div>

      {revising && (
        <ReviseComposePanel
          commentCount={inlineCount}
          nextAttempts={Math.min(task.attempts + 1, 3)}
          target={reviseTarget}
          onCancel={() => setRevising(false)}
          onSend={(direction) => {
            setRevising(false);
            onRevise(task.id, direction);
          }}
        />
      )}

      <div style={actionBar}>
        <span style={{ marginRight: "auto", color: "var(--text-3)", fontSize: 11 }}>
          {inlineCount > 0 ? `${inlineCount} comments` : "no comments"}
          {gated ? " · at gate" : ""}
        </span>
        <Button variant="danger" disabled={!gated} onClick={() => onReject(task.id)}>
          reject
        </Button>
        <Button
          variant={inlineCount > 0 ? "primary" : "default"}
          disabled={!gated}
          onClick={() => setRevising(true)}
        >
          revise
        </Button>
        <Button
          variant={inlineCount > 0 ? "default" : "primary"}
          disabled={!gated}
          onClick={() => onApprove(task.id)}
        >
          approve
        </Button>
      </div>
    </div>
  );
}
