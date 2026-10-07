import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { useState } from "react";

const LIST = {
  source: "live" as const,
  cli_version: "2.1.292",
  models: [
    { value: "default", resolved_model: "claude-opus-5-5", display_name: "Default", description: null, effort_levels: ["low", "medium", "high", "xhigh", "max"], supports_auto_mode: true },
    { value: "opus", resolved_model: "claude-opus-5-5", display_name: "Opus 5.5", description: null, effort_levels: ["low", "high", "xhigh"], supports_auto_mode: true },
    { value: "claude-opus-4-6", resolved_model: "claude-opus-4-6", display_name: "Opus 4.6", description: null, effort_levels: ["low", "high"], supports_auto_mode: true },
    { value: "haiku", resolved_model: "claude-haiku-4-5-20251001", display_name: "Haiku 4.5", description: null, effort_levels: [], supports_auto_mode: false },
  ],
};
const refreshMock = vi.fn(async () => LIST);
vi.mock("../../ipc/models", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/models")>()),
  getModelList: vi.fn(async () => LIST),
  refreshModelList: () => refreshMock(),
  onModelListUpdated: vi.fn(async () => () => {}),
}));
const listPluginsMock = vi.fn(async () => [
  { name: "ddd-council", marketplace: "ddd-council", path: "/plugins/ddd-council" },
  { name: "superpowers", marketplace: "official", path: "/plugins/superpowers" },
]);
vi.mock("../../ipc/plugins", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/plugins")>()),
  listPlugins: () => listPluginsMock(),
}));
const listDirMock = vi.fn();
vi.mock("../../ipc/workspace", () => ({ listDir: (p: string) => listDirMock(p) }));
// G5 — mock only regenerateTeamPrompt (the chat seam); keep the rest of the
// pipeline IPC module intact (setPromptBody etc. live in ../draft, not here).
const regenerateTeamPromptMock = vi.fn();
vi.mock("../../ipc/pipeline", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../ipc/pipeline")>()),
  regenerateTeamPrompt: (...a: unknown[]) => regenerateTeamPromptMock(...a),
}));

import { NodeDrawer } from "./NodeDrawer";
import { emptyDraft, addTeam, setTeamEffort } from "../draft";
import type { DraftPipeline } from "../../ipc/pipeline";
import type { SkillEntry } from "../../ipc/skills";

function teamDraft(): DraftPipeline {
  return addTeam(emptyDraft(), "research", "Research");
}

function withModel(model: string): DraftPipeline {
  const d = teamDraft();
  return { ...d, teams: d.teams.map((t) => ({ ...t, runner: { ...t.runner, model } })) };
}

describe("NodeDrawer — team editor round-trips", () => {
  it("is closed when nothing is selected", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId={null} onChange={() => {}} onClose={() => {}} />);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("header names the selected node kind · id", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    expect(screen.getByRole("dialog", { name: /team · research/i })).toBeInTheDocument();
  });

  it("editing the name flows through renameTeam", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("name for research"), { target: { value: "Investigators" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].name).toBe("Investigators");
  });

  it("editing the prompt flows through setPromptBody", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("prompt for research"), { target: { value: "look at the repo" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].prompt_body).toBe("look at the repo");
  });

  it("changing role flows through setTeamRole", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("role for research"), { target: { value: "reviewer" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].role).toBe("reviewer");
  });

  it("editing Scale min/max flows through setTeamWorkers", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("scale max for research"), { target: { value: "4" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].workers).toEqual({ min: 1, max: 4 });
  });

  it("a store node header reads Store · <team-id>", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="store:research" onChange={() => {}} onClose={() => {}} />);
    expect(screen.getByRole("dialog", { name: /store · research/i })).toBeInTheDocument();
  });

  it("a store node drawer edits the owning team's store.capacity", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="store:research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("store capacity for research"), { target: { value: "5" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].store.capacity).toBe(5);
  });

  it("a store node hides the Delete button (derived, not deletable)", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="store:research" onChange={() => {}} onClose={() => {}} onDelete={() => {}} />);
    expect(screen.queryByRole("button", { name: /delete/i })).not.toBeInTheDocument();
  });

  it("editing Store capacity flows through setTeamStoreCapacity", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("store capacity for research"), { target: { value: "16" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].store).toEqual({ capacity: 16 });
  });

  it("selecting a model flows through setTeamModel", async () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    await screen.findByRole("option", { name: "haiku → claude-haiku-4-5-20251001" });
    fireEvent.change(screen.getByLabelText("model for research"), { target: { value: "haiku" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.model).toBe("haiku");
  });
});

describe("NodeDrawer — A4 skill autocomplete on the prompt field", () => {
  const skills: SkillEntry[] = [
    { name: "brainstorming", kind: "skill", namespace: "superpowers", description: "Explore intent", verbs: [], source: "global", qualified: false },
    { name: "ddd-council", kind: "skill", namespace: "ddd-council", description: "council", verbs: ["vet"], source: "project", qualified: false },
  ];

  function ControlledDrawer({ initial = "" }: { initial?: string }) {
    const [draft, setDraft] = useState<DraftPipeline>(() => {
      const d = teamDraft();
      return { ...d, teams: d.teams.map((t) => ({ ...t, prompt_body: initial })) };
    });
    return <NodeDrawer draft={draft} selectedId="research" onChange={setDraft} onClose={() => {}} skills={skills} />;
  }

  it("typing '/' in the prompt opens the popover; selecting inserts the token", async () => {
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { cb(0); return 0; });
    render(<ControlledDrawer />);
    const ta = screen.getByLabelText("prompt for research") as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "/bra" } });
    ta.setSelectionRange(4, 4);
    fireEvent.click(ta); // re-evaluate the trigger with the caret in place
    await waitFor(() => expect(screen.getByRole("listbox")).toBeInTheDocument());
    // Pick the brainstorming row explicitly (the catalog has more than one entry).
    const option = screen.getAllByRole("option").find((o) => o.textContent?.includes("/brainstorming"))!;
    fireEvent.mouseDown(option);
    await waitFor(() => expect((screen.getByLabelText("prompt for research") as HTMLTextAreaElement).value).toBe("/brainstorming "));
    vi.unstubAllGlobals();
  });

  it("picking a plugin's skill in the prompt adds that plugin to the team", async () => {
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => { cb(0); return 0; });
    let latest: DraftPipeline | null = null;
    function Spy() {
      const [draft, setDraft] = useState<DraftPipeline>(teamDraft);
      latest = draft;
      return <NodeDrawer draft={draft} selectedId="research" onChange={setDraft} onClose={() => {}} skills={skills} />;
    }
    render(<Spy />);
    const ta = screen.getByLabelText("prompt for research") as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "/bra" } });
    ta.setSelectionRange(4, 4);
    fireEvent.click(ta);
    await waitFor(() => expect(screen.getByRole("listbox")).toBeInTheDocument());
    const option = screen.getAllByRole("option").find((o) => o.textContent?.includes("/brainstorming"))!;
    fireEvent.mouseDown(option);
    await waitFor(() => expect(latest!.teams[0].scope.plugins).toEqual(["superpowers"]));
    expect(latest!.teams[0].prompt_body).toBe("/brainstorming ");
    vi.unstubAllGlobals();
  });

  it("recognized tokens get the highlight class in the mirror overlay", () => {
    render(<ControlledDrawer initial="run /brainstorming and /nope" />);
    const tinted = document.querySelectorAll('.abp-skill-token[data-recognized="true"]');
    expect(tinted.length).toBe(1);
    expect(tinted[0].textContent).toBe("/brainstorming");
  });
});

describe("NodeDrawer — model and effort pickers", () => {
  it("lists the CLI's models with alias rows showing their resolved model", async () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "opus → claude-opus-5-5" });
    expect(screen.getByRole("option", { name: "claude-opus-4-6" })).toBeInTheDocument();
    expect((screen.getByLabelText("model for research") as HTMLSelectElement).value).toBe("default");
    expect(screen.queryByLabelText(/model override/)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/test model/)).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/budget for/)).not.toBeInTheDocument();
  });

  it("offers Default plus the selected model's levels", async () => {
    render(<NodeDrawer draft={withModel("claude-opus-4-6")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "claude-opus-4-6" });
    const effort = screen.getByLabelText("effort for research");
    expect(within(effort).getAllByRole("option").map((o) => o.textContent)).toEqual(["Default", "low", "high"]);
  });

  it("a model without effort support offers only Default", async () => {
    render(<NodeDrawer draft={withModel("haiku")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "haiku → claude-haiku-4-5-20251001" });
    const effort = screen.getByLabelText("effort for research");
    expect(within(effort).getAllByRole("option").map((o) => o.textContent)).toEqual(["Default"]);
  });

  it("choosing a level saves it; choosing Default removes it", async () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={withModel("opus")} selectedId="research" onChange={onChange} onClose={() => {}} />);
    await screen.findByRole("option", { name: "xhigh" });
    fireEvent.change(screen.getByLabelText("effort for research"), { target: { value: "xhigh" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.effort).toBe("xhigh");
    fireEvent.change(screen.getByLabelText("effort for research"), { target: { value: "" } });
    expect("effort" in onChange.mock.calls.at(-1)?.[0].teams[0].runner).toBe(false);
  });

  it("switching to a model without the saved level snaps to Default and says so", async () => {
    function Host() {
      const [d, setD] = useState(() => setTeamEffort(withModel("opus"), "research", "xhigh"));
      return <NodeDrawer draft={d} selectedId="research" onChange={setD} onClose={() => {}} />;
    }
    render(<Host />);
    await screen.findByRole("option", { name: "claude-opus-4-6" });
    fireEvent.change(screen.getByLabelText("model for research"), { target: { value: "claude-opus-4-6" } });
    expect((screen.getByLabelText("effort for research") as HTMLSelectElement).value).toBe("");
    expect(screen.getByRole("status")).toHaveTextContent(/xhigh.*claude-opus-4-6.*Default/);
  });

  it("the snap note does not follow the drawer to another team", async () => {
    function Host() {
      const [d, setD] = useState(() => {
        const two = addTeam(setTeamEffort(withModel("opus"), "research", "xhigh"), "review", "Review");
        return two;
      });
      const [sel, setSel] = useState("research");
      return (
        <>
          <button onClick={() => setSel("review")}>select review</button>
          <NodeDrawer draft={d} selectedId={sel} onChange={setD} onClose={() => {}} />
        </>
      );
    }
    render(<Host />);
    await screen.findByRole("option", { name: "claude-opus-4-6" });
    fireEvent.change(screen.getByLabelText("model for research"), { target: { value: "claude-opus-4-6" } });
    expect(screen.getByRole("status")).toBeInTheDocument();
    fireEvent.click(screen.getByText("select review"));
    await screen.findByLabelText("model for review");
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("switching to a model that supports the level keeps it with no note", async () => {
    function Host() {
      const [d, setD] = useState(() => setTeamEffort(withModel("opus"), "research", "high"));
      return <NodeDrawer draft={d} selectedId="research" onChange={setD} onClose={() => {}} />;
    }
    render(<Host />);
    await screen.findByRole("option", { name: "claude-opus-4-6" });
    fireEvent.change(screen.getByLabelText("model for research"), { target: { value: "claude-opus-4-6" } });
    expect((screen.getByLabelText("effort for research") as HTMLSelectElement).value).toBe("high");
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("a saved model missing from the list shows as not available", async () => {
    render(<NodeDrawer draft={withModel("claude-gone")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "claude-gone — not available" });
    expect((screen.getByLabelText("model for research") as HTMLSelectElement).value).toBe("claude-gone");
  });

  it("a saved level the model lacks shows as not supported", async () => {
    render(<NodeDrawer draft={setTeamEffort(withModel("haiku"), "research", "high")} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "high — not supported" });
    expect((screen.getByLabelText("effort for research") as HTMLSelectElement).value).toBe("high");
  });

  it("the refresh button refetches and shows the list's source", async () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    expect(await screen.findByText("live")).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("refresh model list"));
    await waitFor(() => expect(refreshMock).toHaveBeenCalled());
  });
});

describe("NodeDrawer — G8 tooltips", () => {
  it("the Role legend has an accessible info affordance with a popover", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    const help = screen.getByRole("button", { name: /help for Role/i });
    fireEvent.click(help);
    expect(screen.getByRole("tooltip")).toBeInTheDocument();
  });
});

describe("NodeDrawer — scope grants and plugins", () => {
  it("each grant checkbox toggles its grant", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    for (const [label, grant] of [
      ["Bash (all commands)", "bash"],
      ["Agent", "agent"],
      ["WebFetch", "web-fetch"],
      ["WebSearch", "web-search"],
      ["Remote git", "remote-git"],
    ]) {
      const box = screen.getByRole("checkbox", { name: `${label} for research` });
      expect(box).not.toBeChecked();
      fireEvent.click(box);
      expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.grants).toEqual([grant]);
    }
  });

  it("a granted grant shows checked and unchecking removes it", () => {
    const onChange = vi.fn();
    const d = teamDraft();
    const draft = { ...d, teams: d.teams.map((t) => ({ ...t, scope: { ...t.scope, grants: ["agent", "bash(git diff:*)"] } })) };
    render(<NodeDrawer draft={draft} selectedId="research" onChange={onChange} onClose={() => {}} />);
    const box = screen.getByRole("checkbox", { name: "Agent for research" });
    expect(box).toBeChecked();
    fireEvent.click(box);
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.grants).toEqual(["bash(git diff:*)"]);
    expect(screen.getByLabelText("bash patterns for research")).toHaveValue("git diff:*");
  });

  it("bash patterns flow through as pattern grants", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("bash patterns for research"), { target: { value: "git diff:*, git log:*" } });
    fireEvent.blur(screen.getByLabelText("bash patterns for research"));
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.grants).toEqual(["bash(git diff:*)", "bash(git log:*)"]);
  });

  it("the bash patterns field keeps what is typed until it loses focus", () => {
    let latest: DraftPipeline | null = null;
    function Spy() {
      const [draft, setDraft] = useState<DraftPipeline>(teamDraft);
      latest = draft;
      return <NodeDrawer draft={draft} selectedId="research" onChange={setDraft} onClose={() => {}} />;
    }
    render(<Spy />);
    const input = screen.getByLabelText("bash patterns for research") as HTMLInputElement;
    for (const v of ["git ", "git diff:*,", "git diff:*, git "]) {
      fireEvent.change(input, { target: { value: v } });
      expect(input.value).toBe(v);
    }
    fireEvent.change(input, { target: { value: "git diff:*, git log:*" } });
    fireEvent.blur(input);
    expect(latest!.teams[0].scope.grants).toEqual(["bash(git diff:*)", "bash(git log:*)"]);
    expect(input.value).toBe("git diff:*, git log:*");
  });

  it("lists the installed plugins and toggles one onto the team", async () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    const box = await screen.findByRole("checkbox", { name: "plugin superpowers for research" });
    fireEvent.click(box);
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.plugins).toEqual(["superpowers"]);
  });

  it("a declared plugin that is not installed still shows, marked", async () => {
    const d = teamDraft();
    const draft = { ...d, teams: d.teams.map((t) => ({ ...t, scope: { ...t.scope, plugins: ["ghost"] } })) };
    render(<NodeDrawer draft={draft} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("checkbox", { name: "plugin superpowers for research" });
    const ghost = screen.getByRole("checkbox", { name: "plugin ghost for research" });
    expect(ghost).toBeChecked();
    expect(screen.getByText(/not installed/)).toBeInTheDocument();
  });

  it("warns when Remote git is granted on a model without auto mode", async () => {
    const draft = withModel("haiku");
    const withGrant = { ...draft, teams: draft.teams.map((t) => ({ ...t, scope: { ...t.scope, grants: ["remote-git"] } })) };
    render(<NodeDrawer draft={withGrant} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    expect(await screen.findByText(/Remote git needs a model with auto mode/)).toBeInTheDocument();
  });

  it("does not warn on a model with auto mode", async () => {
    const draft = withModel("opus");
    const withGrant = { ...draft, teams: draft.teams.map((t) => ({ ...t, scope: { ...t.scope, grants: ["remote-git"] } })) };
    render(<NodeDrawer draft={withGrant} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    await screen.findByRole("option", { name: "haiku → claude-haiku-4-5-20251001" });
    expect(screen.queryByText(/Remote git needs a model with auto mode/)).not.toBeInTheDocument();
  });

  it("the scope help explains removed versus denied without an em dash", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    const tip = screen.getByRole("button", { name: /help for Grants/i });
    fireEvent.click(tip);
    const text = document.body.textContent ?? "";
    expect(text).toMatch(/removed/);
    expect(text).toMatch(/denied/);
  });
});

describe("NodeDrawer — G7 scope picker", () => {
  it("offers the in-app FileTreePicker for reads when a target repo is bound", async () => {
    listDirMock.mockResolvedValueOnce([{ name: "artifacts", path: "/repo/artifacts", is_dir: true }]);
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} targetRepo="/repo" />);
    fireEvent.click(screen.getByLabelText("browse reads for research"));
    await waitFor(() => expect(screen.getByText(/artifacts/)).toBeInTheDocument());
    // selecting stores the repo-relative path
    fireEvent.click(screen.getByLabelText("select artifacts"));
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.reads).toEqual(["artifacts"]);
  });

  it("shows no Browse button for reads without a target repo (text fallback only)", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    expect(screen.queryByLabelText("browse reads for research")).not.toBeInTheDocument();
    expect(screen.getByLabelText("reads for research")).toBeInTheDocument();
  });
});

describe("NodeDrawer — gate editor", () => {
  function gateDraft(): DraftPipeline {
    const d = addTeam(emptyDraft(), "impl", "Impl");
    return { ...d, gates: [{ id: "gate-1", label: "Plan review", downstream: "" }] };
  }

  it("edits the gate label", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={gateDraft()} selectedId="gate-1" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("label for gate-1"), { target: { value: "Spec gate" } });
    expect(onChange.mock.calls.at(-1)?.[0].gates[0].label).toBe("Spec gate");
  });

  it("sets the gate downstream from the node options", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={gateDraft()} selectedId="gate-1" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("downstream for gate-1"), { target: { value: "impl" } });
    expect(onChange.mock.calls.at(-1)?.[0].gates[0].downstream).toBe("impl");
  });
});

describe("NodeDrawer — join editor (Lanes / Quorum / Early-cancel)", () => {
  function joinDraft(): DraftPipeline {
    return { ...emptyDraft(), joins: [{ id: "join-1", waits_for: ["a", "b", "c"], downstream: "" }] };
  }

  it("lists the waited-for lanes", () => {
    render(<NodeDrawer draft={joinDraft()} selectedId="join-1" onChange={() => {}} onClose={() => {}} />);
    const list = screen.getByLabelText("waits for lanes join-1");
    expect(list).toHaveTextContent("a");
    expect(list).toHaveTextContent("b");
  });

  it("sets a quorum", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={joinDraft()} selectedId="join-1" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("quorum for join-1"), { target: { value: "2" } });
    expect(onChange.mock.calls.at(-1)?.[0].joins[0].quorum).toBe(2);
  });

  it("toggles early-cancel on reject", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={joinDraft()} selectedId="join-1" onChange={onChange} onClose={() => {}} />);
    fireEvent.click(screen.getByLabelText("early cancel for join-1"));
    expect(onChange.mock.calls.at(-1)?.[0].joins[0].cancel_on_reject).toBe(true);
  });

  // DD7 interplay: a set quorum governs success and the runtime ignores
  // cancel_on_reject, so the UI disables the early-cancel toggle and shows a hint.
  it("with no quorum: early-cancel is enabled and the DD7 hint is absent", () => {
    render(<NodeDrawer draft={joinDraft()} selectedId="join-1" onChange={() => {}} onClose={() => {}} />);
    expect(screen.getByLabelText("early cancel for join-1")).not.toBeDisabled();
    expect(screen.queryByText(/ignored while a quorum is set/i)).not.toBeInTheDocument();
  });

  it("with a quorum set: early-cancel is disabled and the DD7 hint is shown", () => {
    const withQuorum: DraftPipeline = {
      ...emptyDraft(),
      joins: [{ id: "join-1", waits_for: ["a", "b", "c"], downstream: "", quorum: 2 }],
    };
    render(<NodeDrawer draft={withQuorum} selectedId="join-1" onChange={() => {}} onClose={() => {}} />);
    expect(screen.getByLabelText("early cancel for join-1")).toBeDisabled();
    expect(screen.getByText(/ignored while a quorum is set/i)).toBeInTheDocument();
  });

  it("setting a quorum live disables the early-cancel toggle (controlled)", () => {
    function Host() {
      const [draft, setDraft] = useState<DraftPipeline>(joinDraft);
      return <NodeDrawer draft={draft} selectedId="join-1" onChange={setDraft} onClose={() => {}} />;
    }
    render(<Host />);
    const cancel = screen.getByLabelText("early cancel for join-1");
    expect(cancel).not.toBeDisabled();
    fireEvent.change(screen.getByLabelText("quorum for join-1"), { target: { value: "2" } });
    expect(screen.getByLabelText("early cancel for join-1")).toBeDisabled();
    expect(screen.getByText(/ignored while a quorum is set/i)).toBeInTheDocument();
  });
});

describe("NodeDrawer — G5 regenerate prompt", () => {
  it("hides the regenerate action when no sessionId is provided", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} />);
    expect(screen.queryByLabelText("regenerate prompt for research")).not.toBeInTheDocument();
  });

  it("shows the regenerate action when a sessionId is provided", () => {
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={() => {}} onClose={() => {}} sessionId="sess-1" />);
    expect(screen.getByLabelText("regenerate prompt for research")).toBeInTheDocument();
  });

  it("regenerate calls the chat seam and writes the new prompt body", async () => {
    regenerateTeamPromptMock.mockReset();
    regenerateTeamPromptMock.mockResolvedValueOnce("You investigate the repo and write findings.");
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} sessionId="sess-1" />);
    fireEvent.click(screen.getByLabelText("regenerate prompt for research"));
    await waitFor(() =>
      expect(onChange.mock.calls.at(-1)?.[0].teams[0].prompt_body).toBe("You investigate the repo and write findings."),
    );
    expect(regenerateTeamPromptMock).toHaveBeenCalledWith("sess-1", expect.anything(), "research");
  });

  it("surfaces an error when no prompt comes back (null)", async () => {
    regenerateTeamPromptMock.mockReset();
    regenerateTeamPromptMock.mockResolvedValueOnce(null);
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} sessionId="sess-1" />);
    fireEvent.click(screen.getByLabelText("regenerate prompt for research"));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent(/no prompt/i));
    expect(onChange).not.toHaveBeenCalled();
  });
});
