import React from "react";
import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { SettingsView } from "./SettingsView";
import type { UsageSnapshot } from "../ipc/usage";
import type { WorktreeEntry } from "../ipc/workspace";

const snap: UsageSnapshot = {
  window_total: 66_500_000, window_budget: 190_000_000, window_pct: 0.35, band: "safe",
  burn_per_min: 0, window_secs: 18000, reset_in_secs: null, est_brake_at: null,
  by_team: [], tokens_by_task: {}, braked: false, auto_meter_enabled: false,
};

function baseProps(over: Partial<React.ComponentProps<typeof SettingsView>> = {}) {
  return {
    usage: snap,
    onSetBudget: vi.fn().mockResolvedValue(snap),
    onSetAutoMeter: vi.fn().mockResolvedValue(snap),
    apiKeyPresent: false,
    onSetApiKey: vi.fn().mockResolvedValue(undefined),
    onClearApiKey: vi.fn().mockResolvedValue(undefined),
    gitConfig: { author_name: "", author_email: "" },
    onSaveGitConfig: vi.fn().mockResolvedValue({ author_name: "", author_email: "" }),
    projects: [],
    activeProjectId: null,
    onRemoveProject: vi.fn().mockResolvedValue(undefined),
    onSetTargetRepo: vi.fn().mockResolvedValue(undefined),
    onSetSkillSources: vi.fn().mockResolvedValue(undefined),
    onListWorktrees: vi.fn().mockResolvedValue([]),
    onRemoveWorktree: vi.fn().mockResolvedValue(undefined),
    ...over,
  } as React.ComponentProps<typeof SettingsView>;
}

beforeEach(() => {
  document.documentElement.setAttribute("data-theme", "dark");
});

describe("SettingsView", () => {
  it("renders General and Usage section headings", () => {
    render(<SettingsView {...baseProps()} />);
    expect(screen.getByText(/general/i)).toBeInTheDocument();
    expect(screen.getByText(/usage/i)).toBeInTheDocument();
  });

  it("toggles the theme attribute when the theme control is used", () => {
    render(<SettingsView {...baseProps()} />);
    fireEvent.click(screen.getByRole("button", { name: /light/i }));
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  it("pre-fills the recalibrated default budget when usage is null", () => {
    render(<SettingsView {...baseProps({ usage: null })} />);
    expect((screen.getByLabelText(/window budget/i) as HTMLInputElement).value).toBe("190000000");
  });

  it("calls onSetBudget with the entered number", async () => {
    const onSetBudget = vi.fn().mockResolvedValue(snap);
    render(<SettingsView {...baseProps({ onSetBudget })} />);
    fireEvent.change(screen.getByLabelText(/window budget/i), { target: { value: "5000000" } });
    fireEvent.click(screen.getByRole("button", { name: /save budget/i }));
    await waitFor(() => expect(onSetBudget).toHaveBeenCalledWith(5_000_000));
  });

  it("renders the auto-brake toggle reflecting snapshot state", () => {
    render(<SettingsView {...baseProps({ usage: { ...snap, auto_meter_enabled: false } })} />);
    const toggle = screen.getByRole("checkbox", { name: /auto-brake/i });
    expect(toggle).not.toBeChecked();
  });

  it("calls onSetAutoMeter when the toggle is flipped", async () => {
    const onSetAutoMeter = vi.fn().mockResolvedValue({ ...snap, auto_meter_enabled: true });
    render(<SettingsView {...baseProps({ usage: { ...snap, auto_meter_enabled: false }, onSetAutoMeter })} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /auto-brake/i }));
    await waitFor(() => expect(onSetAutoMeter).toHaveBeenCalledWith(true));
  });

  it("shows the runners api-key section with a save control", () => {
    render(<SettingsView {...baseProps()} />);
    expect(screen.getByText(/runners/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/api key/i)).toBeInTheDocument();
  });

  it("saves the api key via onSetApiKey", async () => {
    const onSetApiKey = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({ onSetApiKey })} />);
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: "sk-xyz" } });
    fireEvent.click(screen.getByRole("button", { name: /save key/i }));
    await waitFor(() => expect(onSetApiKey).toHaveBeenCalledWith("sk-xyz"));
  });

  it("shows the git author section", () => {
    render(<SettingsView {...baseProps()} />);
    expect(screen.getByText(/git/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/author name/i)).toBeInTheDocument();
    expect(screen.getByLabelText(/author email/i)).toBeInTheDocument();
  });

  it("lists projects and exposes remove", () => {
    render(<SettingsView {...baseProps({ projects: [{ id: "p1", name: "Alpha", root_path: "/a", target_repo: null, skill_sources: [], active_pipeline_id: null, created_at: 0, updated_at: 0 }] })} />);
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    // The project row's own remove button (worktrees expander collapsed by default).
    expect(screen.getByRole("button", { name: "remove project Alpha" })).toBeInTheDocument();
  });

  it("project remove is guarded by a confirm that warns files are deleted but not the target repo", async () => {
    const onRemoveProject = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({
      projects: [{ id: "p1", name: "Alpha", root_path: "/a", target_repo: null, skill_sources: [], active_pipeline_id: null, created_at: 0, updated_at: 0 }],
      onRemoveProject,
    })} />);
    // First click reveals the confirm + the destructive warning; nothing removed yet.
    fireEvent.click(screen.getByRole("button", { name: "remove project Alpha" }));
    expect(onRemoveProject).not.toHaveBeenCalled();
    expect(screen.getByText(/deletes this project's files/i)).toBeInTheDocument();
    expect(screen.getByText(/target repo is NOT touched/i)).toBeInTheDocument();
    // Confirm removes.
    fireEvent.click(screen.getByRole("button", { name: "confirm remove Alpha" }));
    await waitFor(() => expect(onRemoveProject).toHaveBeenCalledWith("p1"));
  });

  it("renders a Target repo field per project and saves it (A5)", async () => {
    const onSetTargetRepo = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({
      projects: [{ id: "p1", name: "Alpha", root_path: "/a", target_repo: "/old", skill_sources: [], active_pipeline_id: null, created_at: 0, updated_at: 0 }],
      onSetTargetRepo,
    })} />);
    const input = screen.getByRole("textbox", { name: "Target repo" }) as HTMLInputElement;
    expect(input.value).toBe("/old");
    fireEvent.change(input, { target: { value: "~/new-repo" } });
    // The target-repo row's own save button (the Git section also has a "save").
    // Walk up from the input until we find the row that also holds a save button.
    let row = input.parentElement as HTMLElement;
    while (row && within(row).queryAllByRole("button", { name: "save" }).length === 0) {
      row = row.parentElement as HTMLElement;
    }
    fireEvent.click(within(row).getByRole("button", { name: "save" }));
    await waitFor(() => expect(onSetTargetRepo).toHaveBeenCalledWith("p1", "~/new-repo"));
  });

  it("shows the global skill source as locked and lists project sources (A4)", () => {
    render(<SettingsView {...baseProps({
      projects: [{ id: "p1", name: "Alpha", root_path: "/a", target_repo: null, skill_sources: ["/work/.claude"], active_pipeline_id: null, created_at: 0, updated_at: 0 }],
    })} />);
    const list = screen.getByLabelText("skill sources for p1");
    expect(list).toHaveTextContent("~/.claude");
    expect(list).toHaveTextContent("global · locked");
    expect(list).toHaveTextContent("/work/.claude");
  });

  it("adds a skill source via the folder field (A4)", async () => {
    const onSetSkillSources = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({
      projects: [{ id: "p1", name: "Alpha", root_path: "/a", target_repo: null, skill_sources: [], active_pipeline_id: null, created_at: 0, updated_at: 0 }],
      onSetSkillSources,
    })} />);
    fireEvent.change(screen.getByLabelText("Add a project .claude root"), { target: { value: "/new/.claude" } });
    fireEvent.click(screen.getByRole("button", { name: "add skill source" }));
    await waitFor(() => expect(onSetSkillSources).toHaveBeenCalledWith("p1", ["/new/.claude"]));
  });

  it("removes a skill source (A4)", async () => {
    const onSetSkillSources = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({
      projects: [{ id: "p1", name: "Alpha", root_path: "/a", target_repo: null, skill_sources: ["/work/.claude"], active_pipeline_id: null, created_at: 0, updated_at: 0 }],
      onSetSkillSources,
    })} />);
    fireEvent.click(screen.getByRole("button", { name: "remove skill source /work/.claude" }));
    await waitFor(() => expect(onSetSkillSources).toHaveBeenCalledWith("p1", []));
  });
});

const oneProject = [
  { id: "p1", name: "Alpha", root_path: "/home/u/proj", target_repo: null, skill_sources: [], active_pipeline_id: null, created_at: 0, updated_at: 0 },
];

describe("SettingsView worktrees (S2)", () => {
  it("lists worktrees when the expander is opened", async () => {
    const entries: WorktreeEntry[] = [
      { path: "/home/u/proj/worktrees/T-1", head: "abc", branch: "refs/heads/t1", stale: true },
    ];
    const onListWorktrees = vi.fn().mockResolvedValue(entries);
    render(<SettingsView {...baseProps({ projects: oneProject, onListWorktrees })} />);

    fireEvent.click(screen.getByRole("button", { name: /worktrees/i }));
    await waitFor(() => expect(onListWorktrees).toHaveBeenCalledWith("p1"));
    expect(await screen.findByText("T-1")).toBeInTheDocument();
  });

  it("requires confirm before removing and calls onRemoveWorktree", async () => {
    const entries: WorktreeEntry[] = [
      { path: "/home/u/proj/worktrees/T-1", head: "abc", branch: "", stale: true },
    ];
    const onListWorktrees = vi.fn().mockResolvedValue(entries);
    const onRemoveWorktree = vi.fn().mockResolvedValue(undefined);
    render(<SettingsView {...baseProps({ projects: oneProject, onListWorktrees, onRemoveWorktree })} />);

    fireEvent.click(screen.getByRole("button", { name: /worktrees/i }));
    const label = await screen.findByText("T-1");
    // Walk up to the row flex container that holds the label + its remove button.
    let row = label.parentElement!;
    while (row && within(row).queryByRole("button", { name: "remove" }) === null) {
      row = row.parentElement!;
    }

    fireEvent.click(within(row).getByRole("button", { name: "remove" }));
    expect(onRemoveWorktree).not.toHaveBeenCalled(); // not yet — needs confirm
    fireEvent.click(within(row).getByRole("button", { name: "confirm remove" }));
    await waitFor(() =>
      expect(onRemoveWorktree).toHaveBeenCalledWith("p1", "/home/u/proj/worktrees/T-1"),
    );
  });

  it("shows the empty state when there are no worktrees", async () => {
    const onListWorktrees = vi.fn().mockResolvedValue([]);
    render(<SettingsView {...baseProps({ projects: oneProject, onListWorktrees })} />);
    fireEvent.click(screen.getByRole("button", { name: /worktrees/i }));
    expect(await screen.findByText(/no worktrees to clean up/i)).toBeInTheDocument();
  });
});
