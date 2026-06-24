import { useCallback, useEffect, useState } from "react";
import { Topbar } from "./components/Topbar";
import { ProjectList } from "./components/ProjectList";
import { NewProjectWizard } from "./wizard/NewProjectWizard";
import { PipelineEditor } from "./wizard/PipelineEditor";
import { ViewSwitcher, type View } from "./components/ViewSwitcher";
import { PipelineView } from "./components/PipelineView";
import { useProjects } from "./hooks/useProjects";
import { useTasks } from "./hooks/useTasks";
import { BoardView } from "./components/BoardView";
import { ListView } from "./components/ListView";
import { SettingsView } from "./components/SettingsView";
import { Drawer } from "./components/ui/Drawer";
import { CardDrawer } from "./components/CardDrawer";
import { activateProject, readArtifact, removeProject, workspaceSetTargetRepo, workspaceSetSkillSources, listWorktrees, removeWorktree, getGitConfig, setGitConfig, type GitConfig, type Project } from "./ipc/workspace";
import { setRunnerApiKey, clearRunnerApiKey, getRunnerApiKeyStatus, ANTHROPIC_API_KEY_ID } from "./ipc/secrets";
import { approveGate, reviseGate, rejectGate, brakeOn as brakeOnCmd, brakeOff as brakeOffCmd, brakeState as brakeStateCmd, type Task } from "./ipc/runtime";
import { recordVerdict, addComment } from "./ipc/review";
import { listPipelines, loadPipeline, pipelineToDraft, type DraftPipeline, type Pipeline } from "./ipc/pipeline";
import { useUsage } from "./hooks/useUsage";
import { setBudget, setAutoMeter } from "./ipc/usage";
import { Terminal } from "./components/Terminal";
import { useConversation } from "./hooks/useConversation";
import { useTaskLog } from "./hooks/useTaskLog";

export default function App() {
  const { projects, reload } = useProjects();
  const [wizardOpen, setWizardOpen] = useState(false);
  const [view, setView] = useState<View>("board");
  const [pipeline, setPipeline] = useState<Pipeline | null>(null);
  // A1: the seeded draft for the in-app pipeline editor (null = closed).
  const [editorSeed, setEditorSeed] = useState<DraftPipeline | null>(null);

  const activeProject: Project | null = projects[0] ?? null;

  const { tasks, reload: reloadTasks } = useTasks();
  const liveLog = useTaskLog();
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

  const openTask: Task | null =
    openTaskId != null ? tasks.find((t) => t.id === openTaskId) ?? null : null;

  const { snapshot: usage } = useUsage();

  // S1: Settings — keychain api-key presence + git author config.
  const [apiKeyPresent, setApiKeyPresent] = useState(false);
  const [gitConfig, setGitConfigState] = useState<GitConfig>({ author_name: "", author_email: "" });
  useEffect(() => {
    getRunnerApiKeyStatus(ANTHROPIC_API_KEY_ID).then(setApiKeyPresent).catch(() => {});
    getGitConfig().then(setGitConfigState).catch(() => {});
  }, []);
  const { turns: convoTurns, send: sendToTerminal, streaming: convoStreaming } = useConversation();
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
      <ViewSwitcher active={view} onChange={setView} />
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
            onRemoveProject={async (id) => { await removeProject(id); await reload(); }}
            onSetTargetRepo={async (id, targetRepo) => { await workspaceSetTargetRepo(id, targetRepo); await reload(); }}
            onSetSkillSources={async (id, sources) => { await workspaceSetSkillSources(id, sources); await reload(); }}
            onListWorktrees={(id) => listWorktrees(id)}
            onRemoveWorktree={async (id, path) => { await removeWorktree(id, path); }}
          />
        ) : activeProject == null ? (
          <ProjectList />
        ) : view === "list" ? (
          <ListView
            tasks={tasks}
            tokensByTask={usage?.tokens_by_task ?? {}}
            now={Math.floor(Date.now() / 1000)}
            onOpenCard={setOpenTaskId}
          />
        ) : (
          <BoardView
            pipeline={pipeline}
            tasks={tasks}
            tokensByTask={usage?.tokens_by_task ?? {}}
            onOpenCard={setOpenTaskId}
          />
        )}
      </main>
      <Terminal turns={convoTurns} contextLine={terminalContext} onSend={sendToTerminal} streaming={convoStreaming} />
      <Drawer open={openTask != null} onClose={() => setOpenTaskId(null)}>
        {openTask && (
          <CardDrawer
            task={openTask}
            artifactMarkdown={artifactMarkdown}
            logText={liveLog.logFor(openTask.id)}
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
        />
      )}
      <NewProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
      />
    </div>
  );
}
