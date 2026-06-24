import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { useState } from "react";
import { SkillAutocomplete } from "./SkillAutocomplete";
import type { SkillEntry } from "../../ipc/skills";

function entry(over: Partial<SkillEntry> = {}): SkillEntry {
  return {
    name: "brainstorming",
    kind: "skill",
    namespace: "superpowers",
    description: "Explore intent",
    verbs: [],
    source: "global",
    qualified: false,
    ...over,
  };
}

const CATALOG: SkillEntry[] = [
  entry({ name: "brainstorming", namespace: "superpowers", description: "Explore intent" }),
  entry({ name: "ddd-council", namespace: "ddd-council", verbs: ["vet", "critique"], description: "DDD council" }),
  entry({ name: "init-session", namespace: null, kind: "command", source: "project", description: "load context" }),
];

// rAF in jsdom: run callbacks synchronously so caret-reevaluation resolves.
beforeEachRaf();
function beforeEachRaf() {
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });
}

function Harness({ skills = CATALOG, initial = "" }: { skills?: SkillEntry[]; initial?: string }) {
  const [v, setV] = useState(initial);
  return <SkillAutocomplete value={v} onChange={setV} skills={skills} aria-label="prompt for t1" />;
}

function typeSlash(ta: HTMLTextAreaElement, text: string) {
  // Simulate the caret being at the end after typing.
  fireEvent.change(ta, { target: { value: text } });
  ta.setSelectionRange(text.length, text.length);
}

describe("SkillAutocomplete", () => {
  it("opens the popover on '/' and filters as typed", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "use /");
    await waitFor(() => expect(screen.getByRole("listbox")).toBeTruthy());
    // all three entries
    expect(screen.getAllByRole("option").length).toBe(3);
    typeSlash(ta, "use /ddd");
    await waitFor(() => expect(screen.getAllByRole("option").length).toBe(1));
    expect(screen.getByRole("option").textContent).toContain("/ddd-council");
  });

  it("inserts the bare token on Enter", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "/bra");
    await waitFor(() => expect(screen.getByRole("listbox")).toBeTruthy());
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(ta.value).toBe("/brainstorming "));
  });

  it("opens a verb cascade for entries with verbs and appends the verb", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "/ddd");
    await waitFor(() => expect(screen.getByRole("option").textContent).toContain("/ddd-council"));
    fireEvent.keyDown(ta, { key: "Enter" }); // select skill → verb cascade
    await waitFor(() => expect(screen.getByRole("listbox", { name: /verbs for ddd-council/ })).toBeTruthy());
    const verbs = screen.getAllByRole("option");
    expect(verbs.map((v) => v.textContent)).toEqual(["/ddd-council vet", "/ddd-council critique"]);
    fireEvent.keyDown(ta, { key: "ArrowDown" }); // → critique
    fireEvent.keyDown(ta, { key: "Enter" });
    await waitFor(() => expect(ta.value).toBe("/ddd-council critique "));
  });

  it("Esc dismisses the skill popover", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "/bra");
    await waitFor(() => expect(screen.getByRole("listbox")).toBeTruthy());
    fireEvent.keyDown(ta, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull());
  });

  it("renders kind + source tags on rows", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "/init");
    await waitFor(() => expect(screen.getByRole("option")).toBeTruthy());
    const row = screen.getByRole("option");
    expect(row.textContent).toContain("command");
    expect(row.textContent).toContain("project");
  });

  it("clicking a row inserts it", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "/bra");
    await waitFor(() => expect(screen.getByRole("option")).toBeTruthy());
    fireEvent.mouseDown(screen.getByRole("option"));
    await waitFor(() => expect(ta.value).toBe("/brainstorming "));
  });

  it("tints recognized tokens in the mirror overlay, not unrecognized ones", () => {
    render(<Harness initial="run /brainstorming and /unknown-thing" />);
    const tinted = document.querySelectorAll('.abp-skill-token[data-recognized="true"]');
    expect(tinted.length).toBe(1);
    expect(tinted[0].textContent).toBe("/brainstorming");
  });

  it("shows a no-match status when the query filters everything out", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    typeSlash(ta, "/zzz-nope");
    await waitFor(() => expect(screen.getByRole("status")).toBeTruthy());
    expect(screen.getByRole("status").textContent).toContain("no skill or command matches");
    expect(screen.queryByRole("listbox")).toBeNull();
    // aria-controls is not dangling when no listbox is present.
    expect(ta.getAttribute("aria-controls")).toBeNull();
  });

  it("sets combobox a11y attributes + aria-activedescendant", async () => {
    render(<Harness />);
    const ta = screen.getByLabelText("prompt for t1") as HTMLTextAreaElement;
    expect(ta.getAttribute("role")).toBe("combobox");
    typeSlash(ta, "/bra");
    await waitFor(() => expect(ta.getAttribute("aria-expanded")).toBe("true"));
    expect(ta.getAttribute("aria-activedescendant")).toBeTruthy();
  });
});
