import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { NodeDrawer } from "./NodeDrawer";
import { emptyDraft, addTeam } from "../draft";
import type { DraftPipeline } from "../../ipc/pipeline";

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
