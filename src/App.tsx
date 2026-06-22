import { useCallback, useEffect, useState } from "react";
import { Topbar } from "./components/Topbar";
import { ProjectList } from "./components/ProjectList";
import { ProjectWizard } from "./components/ProjectWizard";
import { ViewSwitcher, type View } from "./components/ViewSwitcher";
import { PipelineView } from "./components/PipelineView";
import { useProjects } from "./hooks/useProjects";
import { useTasks } from "./hooks/useTasks";
import { BoardView } from "./components/BoardView";
import { Drawer } from "./components/ui/Drawer";
import { CardDrawer } from "./components/CardDrawer";
import { readArtifact, type Project } from "./ipc/workspace";
import { approveGate, reviseGate, rejectGate, type Task } from "./ipc/runtime";
import { recordVerdict, addComment } from "./ipc/review";
import { instantiateTemplate, listPipelines, loadPipeline, type Pipeline } from "./ipc/pipeline";

export default function App() {
  const { projects, reload } = useProjects();
  const [wizardOpen, setWizardOpen] = useState(false);
  const [view, setView] = useState<View>("board");
  const [pipeline, setPipeline] = useState<Pipeline | null>(null);

  const activeProject: Project | null = projects[0] ?? null;

  const { tasks, reload: reloadTasks } = useTasks();
  const [openTaskId, setOpenTaskId] = useState<string | null>(null);

  const openTask: Task | null =
    openTaskId != null ? tasks.find((t) => t.id === openTaskId) ?? null : null;

  // Load the open task's artifact body via the Workspace read_artifact OHS
  // command (D10). Empty until loaded / when the task has no artifact path.
  const [artifactMarkdown, setArtifactMarkdown] = useState("");
  useEffect(() => {
    let cancelled = false;
    const path = openTask?.review_artifact ?? openTask?.parent_artifact ?? null;
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
  }, [openTask, activeProject]);

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
    (_p: Project) => {
      setWizardOpen(false);
      reload();
    },
    [reload],
  );

  // Load (or first-time instantiate) the active project's pipeline whenever the
  // active project changes. v1 instantiates the bundled DDD template on first
  // view so the read-only viewer has a graph to show.
  useEffect(() => {
    let cancelled = false;
    if (!activeProject) {
      setPipeline(null);
      return;
    }
    const root = activeProject.root_path;
    (async () => {
      try {
        const ids = await listPipelines(root);
        const target = ids[0] ?? null;
        const loaded = target
          ? await loadPipeline(root, target)
          : await instantiateTemplate(root, "ddd-spec-plan-impl");
        if (!cancelled) setPipeline(loaded);
      } catch {
        if (!cancelled) setPipeline(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [activeProject]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <Topbar activeProject={activeProject} onNewProject={() => setWizardOpen(true)} />
      <ViewSwitcher active={view} onChange={setView} />
      <main style={{ flex: 1, overflow: "auto" }}>
        {view === "pipeline" ? (
          <PipelineView pipeline={pipeline} />
        ) : activeProject == null ? (
          <ProjectList />
        ) : (
          <BoardView
            pipeline={pipeline}
            tasks={tasks}
            onOpenCard={setOpenTaskId}
          />
        )}
      </main>
      <Drawer open={openTask != null} onClose={() => setOpenTaskId(null)}>
        {openTask && (
          <CardDrawer
            task={openTask}
            artifactMarkdown={artifactMarkdown}
            reviseTarget={reviseTargetFor(openTask)}
            onApprove={handleApprove}
            onRevise={handleRevise}
            onReject={handleReject}
          />
        )}
      </Drawer>
      <ProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
      />
    </div>
  );
}
