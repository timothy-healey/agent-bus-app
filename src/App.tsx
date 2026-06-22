import { useCallback, useEffect, useState } from "react";
import { Topbar } from "./components/Topbar";
import { ProjectList } from "./components/ProjectList";
import { ProjectWizard } from "./components/ProjectWizard";
import { ViewSwitcher, type View } from "./components/ViewSwitcher";
import { PipelineView } from "./components/PipelineView";
import { useProjects } from "./hooks/useProjects";
import { type Project } from "./ipc/workspace";
import { instantiateTemplate, listPipelines, loadPipeline, type Pipeline } from "./ipc/pipeline";

export default function App() {
  const { projects, reload } = useProjects();
  const [wizardOpen, setWizardOpen] = useState(false);
  const [view, setView] = useState<View>("board");
  const [pipeline, setPipeline] = useState<Pipeline | null>(null);

  const activeProject: Project | null = projects[0] ?? null;

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
        {view === "pipeline" ? <PipelineView pipeline={pipeline} /> : <ProjectList />}
      </main>
      <ProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
      />
    </div>
  );
}
