import { useProjects } from "../hooks/useProjects";

export function ProjectList() {
  const { projects, loading } = useProjects();

  if (loading) {
    return <div style={{ padding: "var(--sp-8)", color: "var(--text-3)" }}>Loading projects…</div>;
  }

  if (projects.length === 0) {
    return (
      <div style={{ padding: "var(--sp-8)", color: "var(--text-3)", textAlign: "center" }}>
        No projects yet. Use <strong style={{ color: "var(--text)" }}>New Project</strong> in the topbar.
      </div>
    );
  }

  return (
    <ul style={{ listStyle: "none", padding: 0, margin: 0 }}>
      {projects.map((p) => (
        <li
          key={p.id}
          style={{
            padding: "var(--sp-4) var(--sp-7)",
            borderBottom: "1px solid var(--border)",
          }}
        >
          <div style={{ color: "var(--text)", fontWeight: 500 }}>{p.name}</div>
          <div style={{ color: "var(--text-3)", fontSize: "11px" }}>{p.root_path}</div>
        </li>
      ))}
    </ul>
  );
}
