import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { useState } from "react";
import { NodeDrawer } from "./NodeDrawer";
import { emptyDraft, addTeam } from "../draft";
import type { DraftPipeline } from "../../ipc/pipeline";
import type { SkillEntry } from "../../ipc/skills";

function teamDraft(): DraftPipeline {
  return addTeam(emptyDraft(), "research", "Research");
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

  it("editing Store capacity flows through setTeamStoreCapacity", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("store capacity for research"), { target: { value: "16" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].store).toEqual({ capacity: 16 });
  });

  it("editing the model flows through setTeamModel", () => {
    const onChange = vi.fn();
    render(<NodeDrawer draft={teamDraft()} selectedId="research" onChange={onChange} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText("model for research"), { target: { value: "claude-haiku-4" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.model).toBe("claude-haiku-4");
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

  it("recognized tokens get the highlight class in the mirror overlay", () => {
    render(<ControlledDrawer initial="run /brainstorming and /nope" />);
    const tinted = document.querySelectorAll('.abp-skill-token[data-recognized="true"]');
    expect(tinted.length).toBe(1);
    expect(tinted[0].textContent).toBe("/brainstorming");
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
});
