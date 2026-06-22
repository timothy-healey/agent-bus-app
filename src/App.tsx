import { useCallback, useState } from "react";
import { Topbar } from "./components/Topbar";
import { ProjectList } from "./components/ProjectList";
import { ProjectWizard } from "./components/ProjectWizard";
import { useProjects } from "./hooks/useProjects";
import { type Project } from "./ipc/workspace";

export default function App() {
  const { projects, reload } = useProjects();
  const [wizardOpen, setWizardOpen] = useState(false);

  // For Plan 1, the "active project" is just the newest one in the list.
  // Plan 2 introduces explicit active-project selection.
  const activeProject: Project | null = projects[0] ?? null;

  const onCreated = useCallback((_p: Project) => {
    setWizardOpen(false);
    reload();
  }, [reload]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <Topbar activeProject={activeProject} onNewProject={() => setWizardOpen(true)} />
      <main style={{ flex: 1, overflow: "auto" }}>
        <ProjectList />
      </main>
      <ProjectWizard
        open={wizardOpen}
        onClose={() => setWizardOpen(false)}
        onCreated={onCreated}
      />
    </div>
  );
}
