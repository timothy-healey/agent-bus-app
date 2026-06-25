import { useCallback, useEffect, useState } from "react";
import { Topbar } from "./components/Topbar";
import { ProjectList } from "./components/ProjectList";
import { NewProjectWizard } from "./wizard/NewProjectWizard";
import { PipelineEditor } from "./wizard/PipelineEditor";
import { ViewSwitcher, type View } from "./components/ViewSwitcher";
import { PipelineView } from "./components/PipelineView";
import { useProjects } from "./hooks/useProjects";
import { useTasks } from "./hooks/useTasks";
import { useRuns } from "./hooks/useRuns";
import { useStoreOccupancy } from "./hooks/useStoreOccupancy";
import { BoardView } from "./components/BoardView";
import { ListView } from "./components/ListView";
import { RunSelector } from "./components/RunSelector";
import { SettingsView } from "./components/SettingsView";
import { Drawer } from "./components/ui/Drawer";
import { CardDrawer } from "./components/CardDrawer";
import { activateProject, readArtifact, removeProject, workspaceSetTargetRepo, workspaceSetSkillSources, listWorktrees, removeWorktree, getGitConfig, setGitConfig, type GitConfig, type Project } from "./ipc/workspace";
import { setRunnerApiKey, clearRunnerApiKey, getRunnerApiKeyStatus, ANTHROPIC_API_KEY_ID } from "./ipc/secrets";
import { approveGate, reviseGate, rejectGate, brakeOn as brakeOnCmd, brakeOff as brakeOffCmd, brakeState as brakeStateCmd, startRun as startRunCmd, listInvocations, retryTask, forceAdvance, abandonTask, acceptTask, type Task, type InvocationRow } from "./ipc/runtime";
import { recordVerdict, addComment } from "./ipc/review";
import { listPipelines, loadPipeline, pipelineToDraft, type DraftPipeline, type Pipeline } from "./ipc/pipeline";
import { useUsage } from "./hooks/useUsage";
import { setBudget, setAutoMeter } from "./ipc/usage";
import { Terminal } from "./components/Terminal";
import { useConversation } from "./hooks/useConversation";
import { useTaskLog } from "./hooks/useTaskLog";
import { useActiveGenerators } from "./hooks/useActiveGenerators";

export default function App() {
  const { projects, reload } = useProjects();
  const [wizardOpen, setWizardOpen] = useState(false);
  const [view, setView] = useState<View>("board");
  const [pipeline, setPipeline] = useState<Pipeline | null>(null);
  // A1: the seeded draft for the in-app pipeline editor (null = closed).
  const [editorSeed, setEditorSeed] = useState<DraftPipeline | null>(null);

  // The selected active project (G13/G14). Defaults to the newest (projects[0],
  // newest-first) and falls back to it whenever the selection no longer exists
  // (e.g. after a delete) — so the switcher can move between projects and a
  // delete lands on the next project (or the empty state when none remain).
  const [selectedProjectId, setSelectedProjectId] = useState<string | null>(null);
  // Surfaced delete failure (every delete site — switcher, AuthoringLayout,
  // Settings — awaits handleDeleteProject; without this it's an unhandled rejection).
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const activeProject: Project | null =
    projects.find((p) => p.id === selectedProjectId) ?? projects[0] ?? null;

  // Switch the active project (G14): select it locally + (re)activate its runtime
  // so `/inject` targets it (the activate path is idempotent — R6).
  const selectProject = useCallback((id: string) => {
    setSelectedProjectId(id);
    void activateProject(id);
  }, []);

  // Delete a project (G13): remove it, reload, then let the active-project
  // fallback land on the next project (or the empty state when none remain).
  const handleDeleteProject = useCallback(
    async (id: string) => {
      setDeleteError(null);
      try {
        await removeProject(id);
        // Only reset selection + reload on success; the activeProject fallback
        // (?? projects[0]) lands on the next project (or empty when none remain).
        setSelectedProjectId((cur) => (cur === id ? null : cur));
        reload();
      } catch (e) {
        // Surface the failure instead of throwing an unhandled rejection back
        // through whichever site awaited us.
        setDeleteError(e instanceof Error ? e.message : String(e));
      }
    },
    [reload],
  );

  const { tasks, reload: reloadTasks, tasksByRun } = useTasks();
  // ④e: the board is run-scoped. useRuns tracks the project's runs + the selected
  // run (defaults to the active/newest run, follows fresh runs when unpinned);
  // useStoreOccupancy feeds the lane "n/cap" indicators for that run.
  const { runs, selectedRun, activeRun, select: selectRun, loading: runsLoading } = useRuns(activeProject?.id ?? null);
  const { occupancy } = useStoreOccupancy(selectedRun?.id ?? null);
  const [starting, setStarting] = useState(false);
  // Cards shown on the board/list are scoped to the selected run. Before any run
  // exists (or for legacy run-less tasks) the board simply shows nothing.
  const scopedTasks = selectedRun ? tasksByRun.get(selectedRun.id) ?? [] : [];

  async function handleStartRun() {
    setStarting(true);
    try {
      const run = await startRunCmd();
      // land on the new run immediately; the run-changed event also refetches.
      selectRun(run.id);
    } catch {
      /* a start failure surfaces via the absence of a new run; keep the UI calm */
    } finally {
      setStarting(false);
    }
  }

  const liveLog = useTaskLog();
  const activeGenerators = useActiveGenerators(selectedRun?.id ?? null);
  const [openTaskId, setOpenTaskId] = useState<string | null>(null);
  // A lineage click overrides which artifact the pane shows (D5: single pane).
  const [lineagePath, setLineagePath] = useState<string | null>(null);
  // B2: two chosen artifact paths to compare side-by-side, and their loaded
  // bodies (read via the same read_artifact OHS command as the single pane).
  const [comparePaths, setComparePaths] = useState<{ a: string; b: string } | null>(null);
  const [compareMarkdown, setCompareMarkdown] = useState<{ left: string; right: string } | null>(null);
  useEffect(() => {
    setLineagePath(null);
    setComparePaths(null);
    setCompareMarkdown(null);
  }, [openTaskId]);

  const openTask: Task | null = (() => {
    if (openTaskId == null) return null;
    const real = tasks.find((t) => t.id === openTaskId);
    if (real) return real;
    // A `gen:` id has no DB row — synthesise a minimal running source Task so the
    // drawer opens on the live-log tab; other tabs show their empty states.
    if (openTaskId.startsWith("gen:")) {
      const stage = openTaskId.split(":")[2] ?? "source";
      return {
        id: openTaskId,
        project_id: activeProject?.id ?? "",
        pipeline: pipeline?.id ?? "",
        topic: `${stage} · scanning…`,
        target_repo: null,
        target_scope: null,
        current_stage: stage,
        state: "running",
        attempts: 1,
        parent_artifact: null,
        review_artifact: null,
        created_at: 0,
        updated_at: 0,
        run_id: selectedRun?.id ?? null,
        item_key: null,
      } as Task;
    }
    return null;
  })();

  const { snapshot: usage } = useUsage();

  // S1: Settings — keychain api-key presence + git author config.
  const [apiKeyPresent, setApiKeyPresent] = useState(false);
  const [gitConfig, setGitConfigState] = useState<GitConfig>({ author_name: "", author_email: "" });
  useEffect(() => {
    getRunnerApiKeyStatus(ANTHROPIC_API_KEY_ID).then(setApiKeyPresent).catch(() => {});
    getGitConfig().then(setGitConfigState).catch(() => {});
  }, []);
  const { turns: convoTurns, send: sendToTerminal, streaming: convoStreaming, busy: convoBusy } = useConversation();
  const terminalContext = pipeline
    ? `${pipeline.name} + ${pipeline.teams.length} teams`
    : "no active pipeline";
  const [brake, setBrake] = useState<{ on: boolean; reason: string | null }>({ on: false, reason: null });

  useEffect(() => {
    brakeStateCmd().then(setBrake).catch(() => {});
  }, []);
  useEffect(() => {
    if (usage) brakeStateCmd().then(setBrake).catch(() => {});
  }, [usage?.braked]);

  async function toggleBrake(next: boolean) {
    const s = next ? await brakeOnCmd("manual") : await brakeOffCmd();
    setBrake(s);
  }

  // L3: the open card's invocation audit trail (newest-first) — drives the
  // CardDrawer history panel + the failure-vs-handoff classification. Reloaded
  // whenever the open card or the task data changes (an action settles a row).
  const [invocations, setInvocations] = useState<InvocationRow[]>([]);
  useEffect(() => {
    let cancelled = false;
    if (!openTaskId) {
      setInvocations([]);
      return;
    }
    (async () => {
      try {
        const rows = await listInvocations(openTaskId);
        if (!cancelled) setInvocations(rows);
      } catch {
        // a load failure leaves the trail empty (the panel shows its empty state).
        if (!cancelled) setInvocations([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [openTaskId, tasks]);

  // L2: a surfaced recovery-action failure (catch → dismissible alert, mirroring
  // the project-delete fix) so a failed action never throws an unhandled rejection.
  const [actionError, setActionError] = useState<string | null>(null);

  // Run an L2 recovery command, surfacing any failure as a dismissible alert and
  // refreshing the board on success. Keeps the card open (the operator sees the
  // state change in place) unless the caller closes it.
  const runRecovery = useCallback(
    async (fn: (taskId: string) => Promise<unknown>, taskId: string) => {
      setActionError(null);
      try {
        await fn(taskId);
        reloadTasks();
      } catch (e) {
        setActionError(e instanceof Error ? e.message : String(e));
      }
    },
    [reloadTasks],
  );

  const handleRetry = useCallback((taskId: string) => runRecovery(retryTask, taskId), [runRecovery]);
  const handleForceAdvance = useCallback((taskId: string) => runRecovery(forceAdvance, taskId), [runRecovery]);
  const handleAbandon = useCallback(
    (taskId: string) => runRecovery(async (id) => { await abandonTask(id); setOpenTaskId(null); }, taskId),
    [runRecovery],
  );
  const handleAccept = useCallback(
    (taskId: string) => runRecovery(async (id) => { await acceptTask(id); setOpenTaskId(null); }, taskId),
    [runRecovery],
  );

  // Load the open task's artifact body via the Workspace read_artifact OHS
  // command (D10). Empty until loaded / when the task has no artifact path.
  const [artifactMarkdown, setArtifactMarkdown] = useState("");
  useEffect(() => {
    let cancelled = false;
    const path = lineagePath ?? openTask?.review_artifact ?? openTask?.parent_artifact ?? null;
    if (!openTask || !activeProject || !path) {
      setArtifactMarkdown("");
      return;
    }
    (async () => {
      try {
        const md = await readArtifact(activeProject.id, path);
        if (!cancelled) setArtifactMarkdown(md);
      } catch {
        if (!cancelled) setArtifactMarkdown("");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [openTask, activeProject, lineagePath]);

  // B2: load both chosen artifact bodies when comparePaths is set. Reuses the
  // Workspace read_artifact OHS command — no new backend edge.
  useEffect(() => {
    let cancelled = false;
    if (!comparePaths || !activeProject) {
      setCompareMarkdown(null);
      return;
    }
    (async () => {
      try {
        const [left, right] = await Promise.all([
          readArtifact(activeProject.id, comparePaths.a),
          readArtifact(activeProject.id, comparePaths.b),
        ]);
        if (!cancelled) setCompareMarkdown({ left, right });
      } catch {
        if (!cancelled) setCompareMarkdown(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [comparePaths, activeProject]);

  // The upstream writer a gate's revise routes back to: the team whose
  // on_approve points at this gate (best-effort; falls back to a label).
  function reviseTargetFor(task: Task): string {
    if (!pipeline) return "the writer";
    const upstream = pipeline.teams.find((t) => t.outputs?.on_approve === task.current_stage);
    return upstream?.id ?? "the writer";
  }

  async function handleApprove(taskId: string) {
    await recordVerdict(taskId, "approve");
    await approveGate(taskId);
    setOpenTaskId(null);
    reloadTasks();
  }
  async function handleRevise(taskId: string, direction: string) {
    if (direction.trim()) {
      const t = tasks.find((x) => x.id === taskId);
      await addComment({
        taskId,
        artifactPath: t?.review_artifact ?? t?.parent_artifact ?? `artifacts/${taskId}.md`,
        note: direction.trim(),
        kind: "direction",
      });
    }
    await recordVerdict(taskId, "revise");
    await reviseGate(taskId);
    setOpenTaskId(null);
    reloadTasks();
  }
  async function handleReject(taskId: string) {
    await recordVerdict(taskId, "reject");
    await rejectGate(taskId);
    setOpenTaskId(null);
    reloadTasks();
  }

  const onCreated = useCallback(
    (p: Project) => {
      setWizardOpen(false);
      // Land on the freshly-created project (G14 selection).
      setSelectedProjectId(p.id);
      // Trigger RUNTIME activation for the new project so `/inject` targets it
      // (runtime re-activation fix — activation is no longer boot-only). Reload
      // the project list regardless of the activation result.
      void activateProject(p.id).finally(() => reload());
    },
    [reload],
  );

  // Load the active project's pipeline. The wizard is the only new-project path
  // now (it always writes a pipeline), so a project with no pipeline file just
  // shows the empty viewer. Extracted as a callback so save-edits can reload (A1).
  const reloadPipeline = useCallback(async () => {
    if (!activeProject) {
      setPipeline(null);
      return;
    }
    const root = activeProject.root_path;
    try {
      const ids = await listPipelines(root);
      const target = ids[0] ?? null;
      const loaded = target ? await loadPipeline(root, target) : null;
      setPipeline(loaded);
    } catch {
      setPipeline(null);
    }
  }, [activeProject]);

  useEffect(() => {
    reloadPipeline();
  }, [reloadPipeline]);

  // A1: open the in-app editor seeded from the active pipeline (read its prompt
  // bodies back via pipeline_to_draft_cmd). On failure (e.g. pipeline missing on
  // disk), stay on the viewer.
  async function openEditor() {
    if (!activeProject || !pipeline) return;
    try {
      const seed = await pipelineToDraft(activeProject.id, activeProject.root_path, pipeline.id);
      setEditorSeed(seed);
    } catch {
      /* opening the editor failed; stay on the viewer */
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <Topbar
        activeProject={activeProject}
        onNewProject={() => setWizardOpen(true)}
        usage={usage}
        brakeOn={brake.on}
        brakeReason={brake.reason ?? undefined}
        onToggleBrake={toggleBrake}
      />
      {deleteError && (
        <div role="alert" style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", padding: "var(--sp-2) var(--sp-4)", background: "var(--danger-2)", borderBottom: "1px solid var(--danger)", color: "var(--danger)", fontSize: "var(--ts-sm)" }}>
          <span style={{ flex: 1 }}>Couldn't delete project: {deleteError}</span>
          <button aria-label="dismiss error" onClick={() => setDeleteError(null)} style={{ background: "transparent", border: "none", color: "var(--danger)", cursor: "pointer", fontFamily: "inherit" }}>✕</button>
        </div>
      )}
      {actionError && (
        <div role="alert" style={{ display: "flex", alignItems: "center", gap: "var(--sp-2)", padding: "var(--sp-2) var(--sp-4)", background: "var(--danger-2)", borderBottom: "1px solid var(--danger)", color: "var(--danger)", fontSize: "var(--ts-sm)" }}>
          <span style={{ flex: 1 }}>Action failed: {actionError}</span>
          <button aria-label="dismiss action error" onClick={() => setActionError(null)} style={{ background: "transparent", border: "none", color: "var(--danger)", cursor: "pointer", fontFamily: "inherit" }}>✕</button>
        </div>
      )}
      <ViewSwitcher active={view} onChange={setView} />
      {/* ④e: the run selector + Start run control scope the run-scoped views
          (board/list). Hidden on pipeline/settings and when no project is active. */}
      {activeProject != null && (view === "board" || view === "list") && (
        <RunSelector
          runs={runs}
          selectedRun={selectedRun}
          activeRun={activeRun}
          onSelect={selectRun}
          onStartRun={handleStartRun}
          braked={brake.on}
          onStop={() => toggleBrake(true)}
          onResume={() => toggleBrake(false)}
          starting={starting}
          loading={runsLoading}
        />
      )}
      <main style={{ flex: 1, overflow: "auto" }}>
        {view === "pipeline" ? (
          <PipelineView pipeline={pipeline} onEdit={activeProject && pipeline ? openEditor : undefined} />
        ) : view === "settings" ? (
          <SettingsView
            usage={usage}
            onSetBudget={setBudget}
            onSetAutoMeter={setAutoMeter}
            apiKeyPresent={apiKeyPresent}
            onSetApiKey={async (k) => { await setRunnerApiKey(ANTHROPIC_API_KEY_ID, k); setApiKeyPresent(true); }}
            onClearApiKey={async () => { await clearRunnerApiKey(ANTHROPIC_API_KEY_ID); setApiKeyPresent(false); }}
            gitConfig={gitConfig}
            onSaveGitConfig={async (n, e) => { const c = await setGitConfig(n, e); setGitConfigState(c); return c; }}
            projects={projects}
            activeProjectId={activeProject?.id ?? null}
            onRemoveProject={handleDeleteProject}
            onSetTargetRepo={async (id, targetRepo) => { await workspaceSetTargetRepo(id, targetRepo); await reload(); }}
            onSetSkillSources={async (id, sources) => { await workspaceSetSkillSources(id, sources); await reload(); }}
            onListWorktrees={(id) => listWorktrees(id)}
            onRemoveWorktree={async (id, path) => { await removeWorktree(id, path); }}
          />
        ) : activeProject == null ? (
          <ProjectList />
        ) : view === "list" ? (
          <ListView
            tasks={scopedTasks}
            tokensByTask={usage?.tokens_by_task ?? {}}
            now={Math.floor(Date.now() / 1000)}
            onOpenCard={setOpenTaskId}
          />
        ) : (
          <BoardView
            pipeline={pipeline}
            tasks={scopedTasks}
            occupancy={occupancy}
            hasRun={selectedRun != null}
            tokensByTask={usage?.tokens_by_task ?? {}}
            activeGenerators={activeGenerators}
            onOpenCard={setOpenTaskId}
          />
        )}
      </main>
      <Terminal turns={convoTurns} contextLine={terminalContext} onSend={sendToTerminal} streaming={convoStreaming} pending={convoBusy} />
      <Drawer open={openTask != null} onClose={() => setOpenTaskId(null)}>
        {openTask && (
          <CardDrawer
            // Remount per task so the at-mount `initialTab` contract is explicit
            // (switching cards picks up the new task's default tab) (M5).
            key={openTask.id}
            task={openTask}
            initialTab={openTask.id.startsWith("gen:") ? "live log" : undefined}
            artifactMarkdown={artifactMarkdown}
            logText={liveLog.logFor(openTask.id)}
            logSegments={liveLog.segmentsFor(openTask.id)}
            reviseTarget={reviseTargetFor(openTask)}
            onOpenArtifact={setLineagePath}
            compareMarkdown={compareMarkdown}
            compareLabels={
              comparePaths
                ? {
                    left: comparePaths.a.split("/").pop() ?? comparePaths.a,
                    right: comparePaths.b.split("/").pop() ?? comparePaths.b,
                  }
                : undefined
            }
            onCompare={(a, b) => setComparePaths({ a, b })}
            onExitCompare={() => {
              setComparePaths(null);
              setCompareMarkdown(null);
            }}
            onApprove={handleApprove}
            onRevise={handleRevise}
            onReject={handleReject}
            invocations={invocations}
            pipeline={pipeline}
            onRetry={handleRetry}
            onForceAdvance={handleForceAdvance}
            onAbandon={handleAbandon}
            onAccept={handleAccept}
          />
        )}
      </Drawer>
      {editorSeed && activeProject && (
        <PipelineEditor
          projectId={activeProject.id}
          seed={editorSeed}
          onClose={() => setEditorSeed(null)}
          onSaved={() => {
            setEditorSeed(null);
            reloadPipeline();
          }}
          projects={projects}
          onSelectProject={(id) => { setEditorSeed(null); selectProject(id); }}
          onDeleteProject={handleDeleteProject}
        />
      )}
      <NewProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
        projects={projects}
        activeProjectId={activeProject?.id ?? null}
        onSelectProject={(id) => { setWizardOpen(false); selectProject(id); }}
        onDeleteProject={handleDeleteProject}
      />
    </div>
  );
}
