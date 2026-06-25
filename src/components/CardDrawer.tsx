import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { InvocationRow, Task } from "../ipc/runtime";
import type { Pipeline } from "../ipc/pipeline";
import { useComments } from "../hooks/useComments";
import { ArtifactView } from "./ArtifactView";
import { CommentRail } from "./CommentRail";
import { ReviseComposePanel } from "./ReviseComposePanel";
import { LineageTab } from "./LineageTab";
import { CompareView, type ComparePane } from "./CompareView";
import { Button } from "./ui/Button";
import { classifyNeedsHuman } from "../lib/classifyNeedsHuman";
import { outcomeLabel, isErrorOutcome } from "../lib/outcomeLabel";
import { formatAge } from "../lib/age";

type Tab = "artifact" | "live log" | "review" | "lineage" | "history";

export interface CardDrawerProps {
  task: Task;
  artifactMarkdown: string;
  /// log text for the live-log tab (v1: settled output, may be empty).
  logText?: string;
  /// Ordered tagged live-log segments (B). Output renders as prose, thinking
  /// dimmed/italic with a marker. When omitted, the drawer falls back to `logText`.
  logSegments?: { kind: "output" | "thinking"; text: string }[];
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
  /// The task's invocation audit trail, newest-first (L3). Drives the history
  /// panel, the headline reason line, and the failure-vs-handoff classification.
  /// Defaults to empty (a card with no recorded invocations).
  invocations?: InvocationRow[];
  /// The active pipeline — the classifier reads its terminal escalation node to
  /// tell a hand-off from a failure. May be null before it loads.
  pipeline?: Pipeline | null;
  /// `now` in epoch seconds for deterministic invocation ages (defaults to the
  /// wall clock; tests pass a fixed value).
  now?: number;
  /// L2 recovery actions on a `needs_human` card. Optional so existing gated-only
  /// hosts keep working; absent handlers simply hide their buttons.
  onRetry?: (taskId: string) => void;
  onForceAdvance?: (taskId: string) => void;
  onAbandon?: (taskId: string) => void;
  onAccept?: (taskId: string) => void;
  /// The tab to open on (C6). A `gen:` generator card opens on `"live log"`;
  /// otherwise the drawer defaults to the artifact tab.
  initialTab?: Tab;
}

export function CardDrawer({
  task,
  artifactMarkdown,
  logText,
  logSegments = [],
  reviseTarget = "the writer",
  onOpenArtifact,
  compareMarkdown,
  compareLabels,
  onCompare,
  onExitCompare,
  onApprove,
  onRevise,
  onReject,
  invocations = [],
  pipeline = null,
  now = Math.floor(Date.now() / 1000),
  onRetry,
  onForceAdvance,
  onAbandon,
  onAccept,
  initialTab,
}: CardDrawerProps) {
  // For a synthetic `gen:` task this fallback is a dead path (e.g.
  // `artifacts/gen:R-1:research.md`) — useComments short-circuits on a `gen:`
  // id and never lists/adds/persists against it (M4), so it's harmless.
  const artifactPath = task.review_artifact ?? task.parent_artifact ?? `artifacts/${task.id}.md`;
  // Pass the viewed artifact body so the hook re-anchors comments addressed in
  // this version (B1); absent a body it falls back to v1 carry-over.
  const { reanchored, add, remove } = useComments(task.id, artifactPath, artifactMarkdown);
  const [tab, setTab] = useState<Tab>(initialTab ?? "artifact");
  const [revising, setRevising] = useState(false);
  const [confirmingAbandon, setConfirmingAbandon] = useState(false);
  const [activeComment, setActiveComment] = useState<string | undefined>();
  const logRef = useRef<HTMLPreElement>(null);

  const inlineCount = reanchored.filter((c) => c.kind === "inline").length;
  const gated = task.state === "gated";
  const needsHuman = task.state === "needs_human";
  // The classification frames the needs_human card: a failure escalation offers
  // recovery; a clean hand-off offers accept/send-back.
  const kind = needsHuman ? classifyNeedsHuman(invocations, pipeline) : null;
  const latest = invocations[0];
  // The card's headline reason line: the noun state (DESIGN copy: "needs you" /
  // "ready for you") plus the latest invocation's humanized outcome + stage.
  const stateLabel = kind === "handoff" ? "ready for you" : "needs you";
  const reason = latest
    ? `${stateLabel} · ${outcomeLabel(latest.outcome)} at ${latest.team_id}`
    : stateLabel;

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
      fontSize: "var(--ts-sm)",
      color: active ? "var(--accent)" : "var(--text-3)",
      background: active ? "var(--surface)" : "transparent",
      border: "none",
      borderRight: "1px solid var(--border)",
      cursor: "pointer",
      fontFamily: "inherit",
    };
  }
  // Derive the live-log tab state from what the drawer already knows (S1). Avoids
  // re-plumbing the telemetry pipeline: running == still producing output. The
  // body the state machine keys off is the OUTPUT-only prose (thinking is display-
  // only); when no tagged segments are supplied it falls back to `logText`.
  const outputText = logSegments.length
    ? logSegments.filter((s) => s.kind === "output").map((s) => s.text).join("")
    : (logText ?? "");
  const logBody = outputText.trim();
  // A worker often streams thinking before any prose (the start of a reasoning
  // turn; the whole life of a `gen:` scan). Drive loading→streaming off whether
  // ANY segment exists (output OR thinking) so that thinking shows immediately
  // instead of the skeleton (M2). Error/settled detection stays output-only.
  const hasAnySegment = logSegments.length > 0 || logBody.length > 0;
  const logState: "loading" | "streaming" | "settled" | "error" | "empty" =
    /\[error\]/i.test(logBody)
      ? "error"
      : task.state === "running"
        ? hasAnySegment
          ? "streaming"
          : "loading"
        : logBody
          ? "settled"
          : "empty";
  const logBlock: CSSProperties = {
    flex: 1,
    margin: 0,
    padding: "14px 18px",
    fontFamily: "var(--font-mono)",
    fontSize: "var(--ts-sm)",
    color: "var(--text-3)",
    overflowY: "auto",
    whiteSpace: "pre-wrap",
    background: "var(--bg-2)",
  };
  // Keep the tail of the live log in view while it streams (S1/B feel). The
  // `segmentsFor` array is mutated in place per delta, so its reference is
  // stable — depend on the segment count AND the last segment's length so the
  // effect re-fires on every new delta. Note: once it fires it always yanks to
  // the bottom (no "scrolled up to read" guard); acceptable for v1.
  useEffect(() => {
    if (tab === "live log" && logState === "streaming" && logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [tab, logState, logSegments.length, logSegments[logSegments.length - 1]?.text.length]);
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
        {/* paddingRight clears the Drawer's absolute close ✕ (top:10/right:12) so
            the stage·attempts text never sits under it (LF30). */}
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6, paddingRight: "var(--sp-7)" }}>
          <code style={{ color: "var(--text-3)", fontSize: 11, fontFamily: "var(--font-mono)" }}>
            {task.item_key ?? ""}
          </code>
          <span style={{ color: "var(--text-3)", fontSize: 11 }}>
            {task.current_stage} · a{task.attempts}
          </span>
        </div>
        <div style={{ fontSize: 14.5, color: "var(--text)" }}>
          {task.topic?.trim() || task.item_key?.trim() || ""}
        </div>
        {needsHuman && (
          <div
            data-testid="reason-line"
            data-kind={kind ?? ""}
            role="status"
            style={{
              marginTop: "var(--sp-2)",
              padding: "var(--sp-1) var(--sp-3)",
              borderRadius: "var(--r-sm)",
              fontSize: "var(--ts-sm)",
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-2)",
              // Full border + background tint (no accent side-stripe): the dot +
              // tint carry the signal, per DESIGN §Anti-patterns.
              border: `1px solid ${kind === "handoff" ? "var(--accent-bd)" : "var(--danger)"}`,
              background: kind === "handoff" ? "var(--accent-2)" : "var(--danger-2)",
              color: kind === "handoff" ? "var(--text-2)" : "var(--danger)",
            }}
          >
            <span
              aria-hidden="true"
              style={{
                flexShrink: 0,
                width: 6,
                height: 6,
                borderRadius: "var(--r-pill)",
                background: kind === "handoff" ? "var(--accent)" : "var(--danger)",
              }}
            />
            {reason}
          </div>
        )}
      </div>

      <div style={tabBar} role="tablist">
        <button className="abp-tab" role="tab" aria-selected={tab === "artifact"} style={tabStyle("artifact")} onClick={() => setTab("artifact")}>
          artifact{inlineCount > 0 ? ` (${inlineCount})` : ""}
        </button>
        <button className="abp-tab" role="tab" aria-selected={tab === "live log"} style={tabStyle("live log")} onClick={() => setTab("live log")}>
          live log
        </button>
        <button className="abp-tab" role="tab" aria-selected={tab === "review"} style={tabStyle("review")} onClick={() => setTab("review")}>
          review
        </button>
        <button className="abp-tab" role="tab" aria-selected={tab === "lineage"} style={tabStyle("lineage")} onClick={() => setTab("lineage")}>
          lineage
        </button>
        <button className="abp-tab" role="tab" aria-selected={tab === "history"} style={tabStyle("history")} onClick={() => setTab("history")}>
          history{invocations.length > 0 ? ` (${invocations.length})` : ""}
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
                comments={reanchored}
                activeId={activeComment}
                onSelect={setActiveComment}
                onDelete={remove}
              />
            </div>
          </>
        )}
        {tab === "live log" && (
          <div data-testid="live-log" data-log-state={logState} style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
            {logState === "loading" && (
              // Skeleton lines while the running worker has produced no output yet
              // (DESIGN.md §States: skeleton, never a spinner in content).
              <div style={{ padding: "16px 18px", display: "flex", flexDirection: "column", gap: 10 }} aria-label="log loading" role="status">
                {[88, 72, 95, 60].map((w, i) => (
                  <div key={i} className="abp-skeleton" style={{ width: `${w}%` }} />
                ))}
              </div>
            )}
            {(logState === "streaming" || logState === "settled") && (
              <pre ref={logRef} style={logBlock} data-testid="live-log-body">
                {logSegments.length
                  ? logSegments.map((s, i) =>
                      s.kind === "thinking" ? (
                        <span
                          key={i}
                          data-log-kind="thinking"
                          style={{ color: "var(--text-3)", fontStyle: "italic" }}
                        >
                          <span aria-hidden="true" style={{ opacity: 0.7 }}>thinking · </span>
                          {s.text}
                        </span>
                      ) : (
                        <span key={i} data-log-kind="output">{s.text}</span>
                      ),
                    )
                  : logBody}
                {logState === "streaming" && (
                  <span className="abp-pulse" style={{ color: "var(--running)" }}>▌</span>
                )}
              </pre>
            )}
            {logState === "error" && (
              <pre style={{ ...logBlock, color: "var(--danger)", borderLeft: "2px solid var(--danger)" }} role="alert">
                {logBody}
              </pre>
            )}
            {logState === "empty" && (
              <div style={{ padding: "16px 18px", color: "var(--text-3)", fontSize: "var(--ts-base)", fontStyle: "italic" }}>
                no log captured for this task yet.
              </div>
            )}
          </div>
        )}
        {tab === "review" && (
          <div style={{ flex: 1, padding: "14px 18px", minWidth: 0 }}>
            {task.review_artifact ? (
              <ArtifactView markdown={artifactMarkdown} onAddComment={() => {}} />
            ) : (
              <div style={{ color: "var(--text-3)", fontSize: "var(--ts-base)", fontStyle: "italic" }}>
                no review artifact yet. gate actions are in the bar below.
              </div>
            )}
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
        {tab === "history" && (
          <div data-testid="history-panel" style={{ flex: 1, overflowY: "auto", padding: "var(--sp-3) var(--sp-4)" }}>
            {invocations.length === 0 ? (
              <div style={{ color: "var(--text-3)", fontSize: "var(--ts-base)", fontStyle: "italic", padding: "var(--sp-1) var(--sp-1)" }}>
                no invocations recorded for this task yet.
              </div>
            ) : (
              <ul aria-label="invocation history" style={{ listStyle: "none", margin: 0, padding: 0, display: "flex", flexDirection: "column", gap: "var(--sp-1)" }}>
                {invocations.map((inv) => {
                  const err = isErrorOutcome(inv.outcome);
                  const settled = inv.settled_at != null;
                  const tokens = inv.input_tokens + inv.output_tokens;
                  return (
                    <li
                      key={inv.invocation_id}
                      style={{
                        display: "flex",
                        flexWrap: "wrap",
                        alignItems: "baseline",
                        gap: "var(--sp-2)",
                        padding: "var(--sp-2) var(--sp-3)",
                        borderRadius: "var(--r-md)",
                        border: "1px solid var(--border)",
                        background: "var(--bg-2)",
                        fontSize: "var(--ts-sm)",
                      }}
                    >
                      <span style={{ color: "var(--text)", fontWeight: 500 }}>{inv.team_id}</span>
                      <span style={{ color: "var(--text-3)" }}>{inv.model}</span>
                      <span style={{ color: "var(--text-3)" }}>a{inv.attempts}</span>
                      <span
                        style={{
                          marginLeft: "auto",
                          color: err ? "var(--danger)" : "var(--text-2)",
                          fontVariantNumeric: "tabular-nums",
                        }}
                      >
                        {settled ? outcomeLabel(inv.outcome) : "in flight"}
                      </span>
                      <span style={{ flexBasis: "100%", color: "var(--text-3)", fontSize: "var(--ts-sm)", fontVariantNumeric: "tabular-nums" }}>
                        {tokens > 0 ? `${tokens.toLocaleString()} tokens · ` : ""}
                        {formatAge(inv.started_at, now)} ago
                      </span>
                    </li>
                  );
                })}
              </ul>
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

      <div style={actionBar} data-testid="action-bar" data-mode={gated ? "gated" : needsHuman ? `needs_human:${kind}` : "read-only"}>
        <span style={{ marginRight: "auto", color: "var(--text-3)", fontSize: 11 }}>
          {inlineCount > 0 ? `${inlineCount} comments` : "no comments"}
          {gated ? " · at gate" : ""}
        </span>

        {gated && (
          <>
            <Button variant="danger" onClick={() => onReject(task.id)}>
              reject
            </Button>
            <Button variant={inlineCount > 0 ? "primary" : "default"} onClick={() => setRevising(true)}>
              revise
            </Button>
            <Button variant={inlineCount > 0 ? "default" : "primary"} onClick={() => onApprove(task.id)}>
              approve
            </Button>
          </>
        )}

        {needsHuman && kind === "failure" && (
          <>
            {onRetry && (
              <Button variant="default" onClick={() => onRetry(task.id)}>
                retry
              </Button>
            )}
            {onForceAdvance && (
              <Button variant="primary" onClick={() => onForceAdvance(task.id)}>
                approve and advance
              </Button>
            )}
            {onAbandon && !confirmingAbandon && (
              <Button variant="danger" onClick={() => setConfirmingAbandon(true)}>
                abandon
              </Button>
            )}
            {onAbandon && confirmingAbandon && (
              <>
                <Button variant="default" onClick={() => setConfirmingAbandon(false)}>
                  cancel
                </Button>
                <Button variant="danger" onClick={() => { setConfirmingAbandon(false); onAbandon(task.id); }}>
                  confirm abandon
                </Button>
              </>
            )}
          </>
        )}

        {needsHuman && kind === "handoff" && (
          <>
            {onRetry && (
              <Button variant="default" onClick={() => onRetry(task.id)}>
                send back
              </Button>
            )}
            {onAccept && (
              <Button variant="primary" onClick={() => onAccept(task.id)}>
                accept
              </Button>
            )}
          </>
        )}

        {!gated && !needsHuman && (
          <span style={{ color: "var(--text-3)", fontSize: 11, fontStyle: "italic" }}>read only</span>
        )}
      </div>
    </div>
  );
}
