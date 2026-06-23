# Agent Bus App — Plan: Wizard polish trio (W1 / W2 / W3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Polish the already-built brainstorming wizard with three independent improvements: **W1** surfaces the backend's existing best-effort validation live in the chat+draft panel; **W2** grows the per-team advanced panel beyond `model` (effort preset, tools, scope reads/writes); **W3** adds human-gate authoring end-to-end (`DraftPipeline` carries gates → `apply_slice` → `to_pipeline` emits them → best-effort validation covers them → the wiring system prompt + Wiring-step UI can add/place a gate).

**Architecture:** All three extend existing seams without changing the runtime. **W1** adds an `issues: Vec<String>` field to the already-returned `TurnResult` (the backend already computes `best_effort_validate` and discards it at `design_session.rs:188`), plus a tiny stateless `best_effort_validate_cmd` so manual draft edits can re-fetch issues from the *backend* (no validation logic in the frontend), rendered as a banner in `ChatDraftPanel`. **W2** is pure frontend: new `draft.ts` controlled-input helpers + advanced-panel fields, same pattern as the existing `setTeamModel`. **W3** threads `gates: Vec<Gate>` through `DraftPipeline`, the `WiringSlice`, `apply_slice`, `to_pipeline` (today hardcodes `gates: vec![]`), `best_effort_validate`, the `WIRING_SYSTEM_PROMPT`, the TS `DraftPipeline`/`draftToPipeline` adapter, and the `WiringStep` UI — `validate.rs` already handles gate-downstream resolution and the no-gate-inside-a-lane rule, so `validate()`/`route()`/Runtime are untouched.

**Tech Stack:** Rust (serde, serde_json, serde_yaml, async-trait, tokio, thiserror, uuid), the `agent_bus_core` kernel (`EffortMode`, `RunnerKind`), the `llm_chat` ACL (`FakeChatRunner`), Tauri 2 composition root, React 18 + TypeScript + Vitest.

**Source backlog:** `docs/v1.1-backlog.md` → "Wizard polish trio" (W1/W2/W3). Builds directly on `plans/2026-06-23-plan-brainstorming-wizard.md` (the shipped wizard) and its spec `docs/superpowers/specs/2026-06-23-brainstorming-wizard-design.md`.

---

## Open design decisions (W3 — operator must rule before implementing W3)

W1 and W2 have no genuine forks; W3 does. **These are surfaced for sign-off, not baked in.** The plan below implements the *recommended* option for each; if the operator rules differently, the noted tasks change.

### DD1 — How does the user add a gate in the Wiring step?

- **Option A — chat slice only.** The wiring chat emits gates in its `WiringSlice`; no dedicated UI affordance. Smallest surface; consistent with how forks/joins are authored today (the Wiring step's draft view is the *read-only* `PipelineView`, all edits flow through chat).
- **Option B — a dedicated "insert gate" affordance in the Wiring-step UI** (e.g. an "Add human gate" control that picks an upstream team + a downstream node, mirroring the controlled-input pattern), *plus* chat.
- **Option C — both, but the UI affordance is the primary path.**

> **Recommendation: Option C (both), UI-primary.** Gates are the wizard's headline human-in-the-loop primitive, and the current Wiring step is chat-only/read-only — a click-to-add affordance is a real usability win and matches W2's controlled-input discipline. The chat path comes "for free" once `WiringSlice` carries gates (Task W3-2/W3-5), so the marginal cost is only the UI control (Task W3-8). If the operator prefers minimal surface, drop Task W3-8 and keep Option A (the chat slice + system prompt still author gates). **Tasks affected by this ruling: W3-8 (UI affordance — skip for Option A).**

### DD2 — Gate placement model (confirm it matches schema + Runtime routing)

A gate has `{ id, label, downstream }`. Its **upstream is implied** by which team's `on_approve` points at the gate id; the gate forwards approved work to `downstream`. *(Confirmed against the real code: `model.rs:108-115` `Gate { id, label, downstream }` with the doc-comment "The gate's upstream is implied by which team's on_approve points at this gate"; `validate.rs:127-138` resolves `gate.downstream` and adds it to `inbound`. Runtime routing already consumes this shape — gates "already work in Runtime" per the brief.)*

> **Recommendation: adopt as-is — no schema change.** Authoring a gate = (1) add a `Gate { id, label, downstream }` to `draft.gates`, and (2) repoint some team's `on_approve` (or a join's `downstream`) at the new gate id. The Wiring-step affordance (DD1 Option C) does exactly this pair in one action. **No fork to resolve here unless the operator wants a different placement model — flagged only for confirmation.**

### DD3 — Does kickoff / one-shot propose gates, or only manual/chat insertion post-kickoff?

- **Option A — manual/chat insertion only.** `kickoff_generate` keeps emitting only a teams slice (its current `KICKOFF_SYSTEM_PROMPT`); gates are added in the Wiring step afterward.
- **Option B — kickoff may also propose gates.** Extend the kickoff prompt to optionally emit a richer slice including gates.

> **Recommendation: Option A (manual/chat post-kickoff only).** Kickoff today produces *only* a team set (`design_session.rs:80-86`); steps 2–4 refine. Keeping kickoff teams-only preserves that clean staging and avoids the model inventing premature human-gates before the user has even seen the teams. Gates are authored in the Wiring step (chat + UI), which is exactly where forks/joins are authored. **Tasks affected: none add kickoff-gate behavior; if the operator picks Option B, add a task extending `KICKOFF_SYSTEM_PROMPT` + a kickoff-emits-a-gate test.**

### Honor the parallel-flow v1 rule (no gates *inside* a lane)

`validate.rs` already enforces this: `check_lane_linear` (`validate.rs:43-64`) walks each fork lane via `on_approve` and returns `LaneNotLinear` if it hits any non-`Team` node — including a gate (test `a_gate_inside_a_lane_is_rejected`, `validate.rs:377-383`). **W3 adds nothing to `validate.rs`'s hard rules**; it only adds a *best-effort* mirror so the wizard surfaces a gate-in-lane issue live before Review (Task W3-4). No change to the hard rule.

---

## Orientation — the real shipped code this plan builds on

Read these before starting; every task references them. **All file contents below were read from the implemented tree (the wizard is already shipped).**

- **`src-tauri/pipeline/src/model.rs`** — `Pipeline { id, name, description, schema_version, teams, gates: Vec<Gate>, escalations, forks, joins }`; `Gate { id, label, downstream }` (`:108-115`); `Routes { on_approve, on_revise, on_reject }` (all `Option<String>`, `outputs` is the YAML key); `NodeKind { Team, Gate, Escalation, Fork, Join }`. `Pipeline.node_ids()` already includes gates.
- **`src-tauri/pipeline/src/draft.rs`** — `DraftPipeline { id, name, description, schema_version, teams: Vec<DraftTeam>, forks, joins, escalations }` (**no `gates` field today** — W3 adds it). `DraftTeam { id, name, prompt_body, runner, scope, outputs, workers }`. `Slice` enum (`#[serde(tag="kind", rename_all="lowercase")]`) variants `Teams(TeamsSlice)` / `Prompt(PromptSlice)` / `Wiring(WiringSlice)`. `WiringSlice { routes, forks, joins }` (**no gates today**). `apply_slice` (`:154-189`). `best_effort_validate` (`:196-244`) — returns `Vec<String>`, builds a `known` node-id set from teams+forks+joins+escalations (**no gates today**). `to_pipeline` (`:257-281`) hardcodes `gates: vec![]`. `prompt_files`, `to_yaml`.
- **`src-tauri/pipeline/src/design_session.rs`** — `TurnResult { reply_text, updated_draft }` (`:120-124`; **W1 adds `issues`**). `design_session_turn` (`:161-190`) computes `let _issues = best_effort_validate(&draft);` then **discards it** (`:188`) — W1 returns it. `WIRING_SYSTEM_PROMPT` (`:101-108`) documents the wiring mini-schema (**no gates today**). `kickoff_generate`, `Step` enum, `extract_json_block`, `parse_slice`.
- **`src-tauri/pipeline/src/validate.rs`** — hard `validate()`. Already resolves `gate.downstream` (`:127-138`, error `UnresolvedGateDownstream`) and rejects a gate inside a lane (`check_lane_linear` → `LaneNotLinear`). **Untouched by this plan.**
- **`src-tauri/pipeline/src/contract_tests.rs`** — serde key-set regression tests. `pipeline_key_set_matches_ts` already lists `gates`. `draft_pipeline_key_set_matches_ts` (`:160`) currently asserts `["id","name","description","schema_version","teams","forks","joins","escalations"]` — **W3 adds `"gates"`**. `gate_key_set_matches_ts` (`:146`) already locks `Gate`.
- **`src-tauri/agent_bus_core/src/runner.rs`** — `EffortMode` (`#[serde(tag="mode", rename_all="kebab-case")]`): `Off / Standard / ExtendedLow / ExtendedHigh / Custom { budget_tokens }`. TS mirror in `src/ipc/pipeline.ts:5-10`.
- **`src-tauri/app/src/lib.rs`** — `DesignSessionState { runner }` (`:155`); commands `kickoff_generate_cmd` (`:161`), `design_session_turn_cmd` (`:171`), `create_project_from_draft` (`:221`); `create_project_from_draft_inner` (`:185`); `generate_handler![...]` registration (`:530-540`) lists the three. `mod design_session_tests` (`:802`). **W1 registers one new command (`best_effort_validate_cmd`).**
- **`src/ipc/pipeline.ts`** — TS mirrors: `EffortMode`, `RunnerConfig`, `Scope`, `Routes`, `Gate`, `DraftTeam`, `DraftPipeline` (**no `gates` today**), `Step`, `TurnResult { reply_text, updated_draft }` (**W1 adds `issues`**), wrappers `kickoffGenerate` / `designSessionTurn` / `createProjectFromDraft`.
- **`src/ipc/pipeline.test.ts`** — IPC contract tests (`designSessionTurn` returns prose).
- **`src/wizard/draft.ts`** — `emptyDraft()` (**no `gates` today**), `defaultTeam`, `addTeam`/`removeTeam`/`renameTeam`/`setPromptBody`/`setTeamModel`. **W2 adds effort/tools/scope helpers; W3 adds gate helpers + `gates: []` to `emptyDraft`.**
- **`src/wizard/ChatDraftPanel.tsx`** — the shared chat-left + draft-right panel. Calls `designSessionTurn`, applies `out.updated_draft`. **W1 renders `out.issues` as a banner + re-fetches issues on manual edit.**
- **`src/wizard/TeamsStep.tsx`** — team cards + advanced panel (model only today, `:30-42`). **W2 extends the advanced panel.**
- **`src/wizard/WiringStep.tsx`** — `draftToPipeline(d)` adapter (hardcodes `gates: []`, `:13`) + `WiringStep` rendering read-only `PipelineView`. **W3 emits gates in the adapter + adds the add-gate UI (DD1 Option C).**
- **`src/wizard/PromptsStep.tsx`**, **`src/wizard/ReviewStep.tsx`**, **`src/wizard/NewProjectWizard.tsx`** — for reference (Review renders `draftToPipeline`; W3 gates show automatically once the adapter emits them).
- **`src/components/PipelineView.tsx`** — read-only viewer; already renders a "Gates (N)" section (`:73-81`). W3 gates appear here automatically.

---

## File structure (created / modified)

```
src-tauri/
├── pipeline/src/
│   ├── design_session.rs   M  W1: TurnResult.issues + return it; W3: WIRING_SYSTEM_PROMPT gains the gates schema + a wiring-with-gate test
│   ├── draft.rs            M  W3: DraftPipeline.gates; WiringSlice.gates; apply_slice gates; to_pipeline emits gates; best_effort_validate covers gates (+ gate-in-lane)
│   └── contract_tests.rs   M  W1: TurnResult key set; W3: draft_pipeline_key_set gains "gates"
├── app/src/
│   └── lib.rs              M  W1: best_effort_validate_cmd + registration; W1 root test for TurnResult.issues
src/
├── ipc/
│   ├── pipeline.ts         M  W1: TurnResult.issues + bestEffortValidate wrapper; W3: DraftPipeline.gates
│   └── pipeline.test.ts    M  W1: designSessionTurn returns issues; bestEffortValidate contract
├── wizard/
│   ├── draft.ts            M  W2: setTeamEffort/setTeamTools/setTeamReads/setTeamWrites; W3: emptyDraft gates + addGate/removeGate/setTeamApprove helpers
│   ├── draft.test.ts       M  W2 + W3 helper unit tests
│   ├── ChatDraftPanel.tsx  M  W1: render issues banner + re-fetch issues on manual edit
│   ├── ChatDraftPanel.test.tsx  M  W1 issues-banner tests
│   ├── TeamsStep.tsx       M  W2: effort select + tools + reads + writes inputs
│   ├── TeamsStep.test.tsx  M  W2 advanced-panel tests
│   ├── WiringStep.tsx      M  W3: draftToPipeline emits gates; add-gate affordance (DD1 Option C)
│   └── WiringStep.test.tsx M  W3 gate render + add-gate tests
DOMAIN.md                   M  (optional, Task FINAL) note gate authoring + live best-effort in the wizard
```

---

## Decisions (resolved ambiguities — autonomous; W3 forks are in "Open design decisions" above)

- **C1 — W1 surfaces the EXISTING backend best-effort result; it never re-implements validation in the frontend.** Two backend touch-points: (a) `design_session_turn` already calls `best_effort_validate(&draft)` and throws the result away (`design_session.rs:188`) — W1 adds `issues: Vec<String>` to `TurnResult` and returns it; (b) for *manual* draft edits (which never hit `design_session_turn`), W1 adds a thin stateless `best_effort_validate_cmd(draft) -> Vec<String>` at the composition root that calls the same `pipeline::draft::best_effort_validate`. The frontend only renders and re-fetches — zero validation logic in TS.
- **C2 — The issues banner lives in `ChatDraftPanel`** (the shared steps-2–4 panel) so all three editing steps get it from one place. It shows after each turn (from `TurnResult.issues`) and re-fetches via `bestEffortValidate(draft)` whenever a manual edit changes the draft (debounced is unnecessary in v1 — fire on each `onDraftChange`). An empty issues list renders nothing.
- **C3 — W2 reuses the existing controlled-input + `mapTeams` pattern verbatim.** Each new advanced field is a pure `draft.ts` helper (`setTeamEffort`, `setTeamTools`, `setTeamReads`, `setTeamWrites`) returning a new `DraftPipeline`, wired to a controlled input in `TeamsStep`'s advanced panel exactly like `setTeamModel`. Effort is a `<select>` over the five `EffortMode` presets; "custom" reveals a numeric budget input. Tools/reads/writes are comma-separated text inputs parsed to `string[]` (trim + drop empties), matching the `Scope` shape.
- **C4 — W3 adds `gates: Vec<Gate>` to `DraftPipeline` and threads it through every seam, but `validate.rs` is untouched.** Hard validation already resolves gate downstream and rejects gates-in-lanes. W3's only validation work is the *best-effort* mirror so the wizard surfaces those issues live before Review.
- **C5 — `WiringSlice` gains `#[serde(default)] gates: Vec<Gate>`** so existing wiring slices (which omit `gates`) still parse — back-compatible. `apply_slice`'s `Slice::Wiring` arm sets `draft.gates = s.gates` alongside `draft.forks`/`draft.joins`.
- **C6 — The add-gate UI affordance (DD1 Option C) does the implied-upstream wiring in one action.** Adding a gate `g` with a chosen upstream team `t` and downstream `d` = push `Gate { id: g, label, downstream: d }` to `draft.gates` AND set `t.outputs.on_approve = g`. A `setTeamApprove(d, teamId, target)` helper makes the route edit, and `addGate(d, id, label, downstream)` adds the gate node. The UI control composes them.
- **C7 — `emptyDraft()` in `draft.ts` and the TS `DraftPipeline` interface gain `gates: []` / `gates: Gate[]`.** The Rust→TS contract test (`draft_pipeline_key_set_matches_ts`) is updated in the same task to keep the boundary locked.

---

## W1 — Live best-effort validation in wizard steps 2–4

### Task W1-1: `TurnResult` carries `issues` (backend returns what it already computes)

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/design_session.rs` `mod tests` (after `design_session_turn_passes_the_step_scoped_dialogue_id`):

```rust
    #[tokio::test]
    async fn turn_returns_best_effort_issues_for_the_resulting_draft() {
        // a draft with one team and NO prompt -> best_effort flags the missing prompt
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("research", "Research"));
        // a reply with no fenced block -> draft unchanged, still missing the prompt
        let runner = FakeChatRunner::new(vec![reply("noted, nothing to change.")]);
        let out = design_session_turn(&runner, "sess-1", Step::Prompts, draft, "hi").await;
        assert!(out.issues.iter().any(|i| i.contains("research") && i.contains("prompt")));
    }

    #[tokio::test]
    async fn turn_returns_empty_issues_for_a_complete_draft() {
        let mut draft = DraftPipeline::empty();
        let mut a = DraftTeam::new("research", "Research");
        a.prompt_body = "investigate".into();
        a.outputs.on_approve = Some("writers".into());
        let mut b = DraftTeam::new("writers", "Writers");
        b.prompt_body = "write".into();
        draft.teams.push(a);
        draft.teams.push(b);
        let runner = FakeChatRunner::new(vec![reply("looks good.")]);
        let out = design_session_turn(&runner, "sess-1", Step::Wiring, draft, "ok").await;
        assert!(out.issues.is_empty());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::tests::turn_returns`
Expected: FAIL — `no field issues on type TurnResult`.

- [ ] **Step 3: Add the field + return it**

In `src-tauri/pipeline/src/design_session.rs`, change the `TurnResult` struct (`:120-124`) to:

```rust
/// The result of one Design Session turn: the assistant's prose, the draft after
/// applying any extracted slice (unchanged if none/invalid), and the live
/// best-effort validation issues for that draft (W1 — surfaced inline; never
/// blocks). Issues mirror `draft::best_effort_validate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnResult {
    pub reply_text: String,
    pub updated_draft: DraftPipeline,
    pub issues: Vec<String>,
}
```

Then in `design_session_turn` (`:161-190`), replace the trailing block (the `let _issues = ...` line through the `TurnResult { ... }` return) with:

```rust
    // best-effort issues are surfaced inline in the wizard (W1); never block.
    let issues = best_effort_validate(&draft);
    TurnResult { reply_text, updated_draft: draft, issues }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::`
Expected: PASS — all `design_session::` tests green (the new two + the existing ones; the existing turn tests construct `TurnResult` only via the function, so they keep compiling).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(pipeline): TurnResult carries best-effort issues (W1 — surface what we already compute)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W1-2: `best_effort_validate_cmd` for manual edits (composition root)

**Files:**
- Modify: `src-tauri/app/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/app/src/lib.rs` `mod design_session_tests` (after `root_turn_applies_a_teams_slice`):

```rust
    #[test]
    fn root_best_effort_reports_issues_for_an_incomplete_draft() {
        let mut d = pipeline::draft::DraftPipeline::empty();
        d.teams.push(pipeline::draft::DraftTeam::new("research", "Research"));
        let issues = pipeline::draft::best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("research") && i.contains("prompt")));
    }
```

(This test pins the function the command wraps; the `#[tauri::command]` wrapper itself is a one-line pass-through that cannot be unit-tested without a Tauri runtime, matching the existing `kickoff_generate_cmd` convention.)

- [ ] **Step 2: Run the test to verify it fails / compiles**

Run: `cd src-tauri && cargo test -p app --lib design_session_tests::root_best_effort`
Expected: PASS immediately for the *logic* test (it calls the existing `best_effort_validate`). Proceed to add the command (next step) and verify the build still compiles + registration is wired.

- [ ] **Step 3: Add the command**

In `src-tauri/app/src/lib.rs`, after `design_session_turn_cmd` (ends `:179`), add:

```rust
/// OHS: recompute best-effort validation for a manually-edited draft (W1). The
/// turn command already returns issues; this serves edits that bypass chat. Pure
/// pass-through to pipeline::draft::best_effort_validate — no state, no chat.
#[tauri::command(rename_all = "snake_case")]
fn best_effort_validate_cmd(draft: pipeline::draft::DraftPipeline) -> Vec<String> {
    pipeline::draft::best_effort_validate(&draft)
}
```

- [ ] **Step 4: Register it**

In the `tauri::generate_handler![...]` block (`:530-540`), add `best_effort_validate_cmd,` after `design_session_turn_cmd,`:

```rust
            kickoff_generate_cmd,
            design_session_turn_cmd,
            best_effort_validate_cmd,
            create_project_from_draft,
```

- [ ] **Step 5: Run the build + test**

Run: `cd src-tauri && cargo test -p app --lib design_session_tests::`
Expected: PASS — the new logic test plus the existing create-from-draft tests; the crate compiles with the new command registered.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/app/src/lib.rs
git commit -m "feat(app): best_effort_validate_cmd for manual draft edits (W1 — backend stays the validation authority)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W1-3: TS mirror — `TurnResult.issues` + `bestEffortValidate` wrapper + contract tests

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Modify: `src/ipc/pipeline.test.ts`
- Modify: `src-tauri/pipeline/src/contract_tests.rs`

- [ ] **Step 1: Write the failing frontend test**

In `src/ipc/pipeline.test.ts`, update the `designSessionTurn` test to assert issues come back, and add a `bestEffortValidate` test. Replace the existing `designSessionTurn` `it(...)` with:

```ts
  it("designSessionTurn passes step + draft + user_message and returns prose + issues", async () => {
    const draft = { id: "p", name: "P", description: "", schema_version: 2, teams: [], gates: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce({ reply_text: "ok", updated_draft: draft, issues: ["draft has no teams yet"] });
    const out = await designSessionTurn("s1", "teams", draft, "add a team");
    expect(invokeMock).toHaveBeenCalledWith("design_session_turn_cmd", {
      session_id: "s1", step: "teams", draft, user_message: "add a team",
    });
    expect(out.reply_text).toBe("ok");
    expect(out.issues).toEqual(["draft has no teams yet"]);
  });

  it("bestEffortValidate passes the draft and returns the issues list", async () => {
    const draft = { id: "p", name: "P", description: "", schema_version: 2, teams: [], gates: [], forks: [], joins: [], escalations: [] };
    invokeMock.mockResolvedValueOnce(["draft has no teams yet"]);
    const issues = await bestEffortValidate(draft);
    expect(invokeMock).toHaveBeenCalledWith("best_effort_validate_cmd", { draft });
    expect(issues).toEqual(["draft has no teams yet"]);
  });
```

Update the import line at the top of the file to:

```ts
import { listPipelines, loadPipeline, kickoffGenerate, designSessionTurn, bestEffortValidate } from "./pipeline";
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/ipc/pipeline.test.ts`
Expected: FAIL — `bestEffortValidate is not exported` / `out.issues` undefined.

- [ ] **Step 3: Update the TS types + add the wrapper**

In `src/ipc/pipeline.ts`, change `TurnResult` (`:111-114`) to:

```ts
export interface TurnResult {
  reply_text: string;
  updated_draft: DraftPipeline;
  issues: string[];
}
```

Add after `designSessionTurn` (ends `:135`):

```ts
export async function bestEffortValidate(draft: DraftPipeline): Promise<string[]> {
  return await invoke<string[]>("best_effort_validate_cmd", { draft });
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/ipc/pipeline.test.ts`
Expected: PASS.

- [ ] **Step 5: Lock the `TurnResult` wire shape (Rust contract test)**

In `src-tauri/pipeline/src/contract_tests.rs`, add (use the existing `keys`/`set` helpers; import `TurnResult` + `DraftPipeline` at the top of the file's `use` block — `DraftPipeline` is already imported):

```rust
/// Locks the TurnResult key set the wizard IPC mirrors (src/ipc/pipeline.ts).
#[test]
fn turn_result_key_set_matches_ts() {
    use crate::design_session::TurnResult;
    let v = serde_json::to_value(TurnResult {
        reply_text: "ok".into(),
        updated_draft: DraftPipeline::empty(),
        issues: vec!["x".into()],
    })
    .unwrap();
    assert_eq!(keys(&v), set(&["reply_text", "updated_draft", "issues"]));
}
```

- [ ] **Step 6: Run the Rust contract test**

Run: `cd src-tauri && cargo test -p pipeline --lib contract_tests::turn_result_key_set`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/ipc/pipeline.ts src/ipc/pipeline.test.ts src-tauri/pipeline/src/contract_tests.rs
git commit -m "feat(web): mirror TurnResult.issues + bestEffortValidate wrapper; lock the wire shape (W1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W1-4: Render the issues banner in `ChatDraftPanel` (turn result + manual-edit re-fetch)

**Files:**
- Modify: `src/wizard/ChatDraftPanel.tsx`
- Modify: `src/wizard/ChatDraftPanel.test.tsx`

- [ ] **Step 1: Write the failing test**

Add to `src/wizard/ChatDraftPanel.test.tsx` (it already mocks `./ipc/pipeline` for `designSessionTurn`; extend the mock to include `bestEffortValidate`). Add these tests inside the existing `describe`:

```ts
  it("shows the issues returned by a chat turn", async () => {
    designSessionTurnMock.mockResolvedValueOnce({
      reply_text: "added",
      updated_draft: emptyDraft(),
      issues: ["draft has no teams yet"],
    });
    render(
      <ChatDraftPanel
        sessionId="s1"
        step="teams"
        draft={emptyDraft()}
        onDraftChange={() => {}}
        renderDraft={() => <div />}
      />,
    );
    fireEvent.change(screen.getByPlaceholderText(/refine this step/i), { target: { value: "hi" } });
    fireEvent.click(screen.getByRole("button", { name: "send" }));
    expect(await screen.findByText(/draft has no teams yet/i)).toBeInTheDocument();
  });

  it("re-fetches issues from the backend when the draft is edited manually", async () => {
    bestEffortValidateMock.mockResolvedValueOnce(["team 'research' has no prompt yet"]);
    let captured: ((d: DraftPipeline) => void) | null = null;
    render(
      <ChatDraftPanel
        sessionId="s1"
        step="prompts"
        draft={addTeam(emptyDraft(), "research", "Research")}
        onDraftChange={() => {}}
        renderDraft={(d, onChange) => {
          captured = onChange;
          return <div />;
        }}
      />,
    );
    captured!(addTeam(emptyDraft(), "research", "Research"));
    expect(await screen.findByText(/has no prompt yet/i)).toBeInTheDocument();
    expect(bestEffortValidateMock).toHaveBeenCalled();
  });
```

At the top of the test file, ensure the mock + imports cover both functions and the helpers used:

```ts
import { addTeam, emptyDraft } from "./draft";
import type { DraftPipeline } from "../ipc/pipeline";

const designSessionTurnMock = vi.fn();
const bestEffortValidateMock = vi.fn().mockResolvedValue([]);
vi.mock("../ipc/pipeline", () => ({
  designSessionTurn: (...a: unknown[]) => designSessionTurnMock(...a),
  bestEffortValidate: (...a: unknown[]) => bestEffortValidateMock(...a),
}));
```

*(If the existing test file already defines a `designSessionTurn` mock, merge — do not duplicate the `vi.mock` factory; add `bestEffortValidate` to the existing factory and add `bestEffortValidateMock`.)*

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/ChatDraftPanel.test.tsx`
Expected: FAIL — the issues text is not rendered; `bestEffortValidate` not called on manual edit.

- [ ] **Step 3: Implement the banner + re-fetch**

Replace `src/wizard/ChatDraftPanel.tsx` body. Key changes: import `bestEffortValidate`; hold an `issues` state; set it from `out.issues` after a turn; wrap `onDraftChange` in a handler that also calls `bestEffortValidate` and updates `issues`; render an issues banner above the draft pane. Full file:

```tsx
import { type ReactNode, useState } from "react";
import { bestEffortValidate, designSessionTurn, type DraftPipeline, type Step } from "../ipc/pipeline";

interface Msg {
  role: "you" | "claude";
  text: string;
}

interface ChatDraftPanelProps {
  sessionId: string;
  step: Step;
  draft: DraftPipeline;
  onDraftChange: (d: DraftPipeline) => void;
  renderDraft: (d: DraftPipeline, onChange: (d: DraftPipeline) => void) => ReactNode;
}

/// The shared two-way-bound panel (layout A): chat left, live-editable draft
/// right. A chat turn emits a slice that updates the draft + is narrated; manual
/// edits mutate the draft so the next turn sends it. W1: best-effort validation
/// issues are surfaced inline — from the turn result, and re-fetched from the
/// backend on every manual edit (the backend stays the validation authority).
export function ChatDraftPanel({ sessionId, step, draft, onDraftChange, renderDraft }: ChatDraftPanelProps) {
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [issues, setIssues] = useState<string[]>([]);

  async function send() {
    const v = value.trim();
    if (!v || busy) return;
    setBusy(true);
    setMsgs((m) => [...m, { role: "you", text: v }]);
    setValue("");
    try {
      const out = await designSessionTurn(sessionId, step, draft, v);
      setMsgs((m) => [...m, { role: "claude", text: out.reply_text }]);
      setIssues(out.issues);
      onDraftChange(out.updated_draft);
    } catch (e) {
      setMsgs((m) => [...m, { role: "claude", text: `[error] ${e instanceof Error ? e.message : String(e)}` }]);
    } finally {
      setBusy(false);
    }
  }

  // Manual edits bypass chat; re-fetch the backend's best-effort issues so the
  // banner stays live without any validation logic in the frontend (W1).
  function handleManualEdit(d: DraftPipeline) {
    onDraftChange(d);
    bestEffortValidate(d).then(setIssues).catch(() => {});
  }

  return (
    <div style={{ display: "flex", gap: "var(--sp-5)", height: "100%" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 280 }}>
        <div style={{ flex: 1, overflowY: "auto", border: "1px solid var(--border)", borderRadius: "var(--r-sm)", padding: "var(--sp-3)" }}>
          {msgs.map((m, i) => (
            <div key={i} style={{ marginBottom: 8 }}>
              <div style={{ color: "var(--text-3)", fontSize: 11 }}>{m.role}</div>
              <div style={{ color: "var(--text)" }}>{m.text}</div>
            </div>
          ))}
        </div>
        <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
          <input
            value={value}
            onChange={(e) => setValue(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") send(); }}
            placeholder="refine this step…"
            style={{ flex: 1, background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" }}
          />
          <button onClick={send} disabled={busy} aria-label="send">Send</button>
        </div>
      </div>
      <div style={{ flex: 1, overflowY: "auto", minWidth: 280 }}>
        {issues.length > 0 && (
          <div
            role="status"
            aria-label="validation issues"
            style={{ marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--accent-bd)", background: "var(--accent-2)", borderRadius: "var(--r-sm)", color: "var(--text-2)", fontSize: 11 }}
          >
            {issues.map((iss, i) => (
              <div key={i}>• {iss}</div>
            ))}
          </div>
        )}
        {renderDraft(draft, handleManualEdit)}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/ChatDraftPanel.test.tsx`
Expected: PASS — the turn's issues render; a manual edit re-fetches and renders.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/ChatDraftPanel.tsx src/wizard/ChatDraftPanel.test.tsx
git commit -m "feat(web): live best-effort issues banner in steps 2-4 (W1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## W2 — Per-team advanced panel beyond `model`

### Task W2-1: `draft.ts` controlled-input helpers (effort / tools / reads / writes)

**Files:**
- Modify: `src/wizard/draft.ts`
- Modify: `src/wizard/draft.test.ts`

- [ ] **Step 1: Write the failing test**

Add to `src/wizard/draft.test.ts`:

```ts
import { setTeamEffort, setTeamTools, setTeamReads, setTeamWrites } from "./draft";

describe("advanced team config helpers (W2)", () => {
  const base = addTeam(emptyDraft(), "research", "Research");

  it("setTeamEffort sets a preset EffortMode", () => {
    const d = setTeamEffort(base, "research", { mode: "extended-high" });
    expect(d.teams[0].runner.effort).toEqual({ mode: "extended-high" });
  });

  it("setTeamEffort sets a custom EffortMode with a budget", () => {
    const d = setTeamEffort(base, "research", { mode: "custom", budget_tokens: 16000 });
    expect(d.teams[0].runner.effort).toEqual({ mode: "custom", budget_tokens: 16000 });
  });

  it("setTeamTools parses a comma list into a trimmed string array", () => {
    const d = setTeamTools(base, "research", "Read, Grep ,  Bash ");
    expect(d.teams[0].scope.tools).toEqual(["Read", "Grep", "Bash"]);
  });

  it("setTeamTools drops empty entries", () => {
    const d = setTeamTools(base, "research", "Read,,");
    expect(d.teams[0].scope.tools).toEqual(["Read"]);
  });

  it("setTeamReads / setTeamWrites set scope.reads / scope.writes", () => {
    const d1 = setTeamReads(base, "research", "src/**, docs/**");
    expect(d1.teams[0].scope.reads).toEqual(["src/**", "docs/**"]);
    const d2 = setTeamWrites(base, "research", "artifacts/**");
    expect(d2.teams[0].scope.writes).toEqual(["artifacts/**"]);
  });
});
```

*(`addTeam`/`emptyDraft` are already imported in `draft.test.ts`; if not, add them to the existing import.)*

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: FAIL — the four setters are not exported.

- [ ] **Step 3: Implement the helpers**

Add to `src/wizard/draft.ts` (after `setTeamModel`). Import `EffortMode` from the IPC types:

Change the top import line to:

```ts
import type { DraftPipeline, DraftTeam, EffortMode } from "../ipc/pipeline";
```

Then append:

```ts
/// Parse a comma-separated input into a trimmed, non-empty string list (the
/// shape Scope.tools/reads/writes use).
function parseCsv(raw: string): string[] {
  return raw.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
}

export function setTeamEffort(d: DraftPipeline, id: string, effort: EffortMode): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, runner: { ...t.runner, effort } } : t));
}

export function setTeamTools(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, scope: { ...t.scope, tools: parseCsv(raw) } } : t));
}

export function setTeamReads(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, scope: { ...t.scope, reads: parseCsv(raw) } } : t));
}

export function setTeamWrites(d: DraftPipeline, id: string, raw: string): DraftPipeline {
  return mapTeams(d, (t) => (t.id === id ? { ...t, scope: { ...t.scope, writes: parseCsv(raw) } } : t));
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/draft.ts src/wizard/draft.test.ts
git commit -m "feat(web): draft helpers for effort/tools/scope (W2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W2-2: Extend the advanced panel in `TeamsStep`

**Files:**
- Modify: `src/wizard/TeamsStep.tsx`
- Modify: `src/wizard/TeamsStep.test.tsx`

- [ ] **Step 1: Write the failing test**

Add to `src/wizard/TeamsStep.test.tsx` `describe` (the existing "the advanced panel edits the model" test stays):

```ts
  it("the advanced panel sets an effort preset", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/effort for research/i), { target: { value: "extended-high" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.effort).toEqual({ mode: "extended-high" });
  });

  it("choosing custom effort reveals a budget input that sets budget_tokens", () => {
    const d = { ...addTeam(emptyDraft(), "research", "Research") };
    d.teams[0].runner.effort = { mode: "custom", budget_tokens: 12000 };
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/budget for research/i), { target: { value: "20000" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].runner.effort).toEqual({ mode: "custom", budget_tokens: 20000 });
  });

  it("the advanced panel edits tools / reads / writes", () => {
    const d = addTeam(emptyDraft(), "research", "Research");
    const onChange = vi.fn();
    render(<TeamsStep draft={d} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /advanced research/i }));
    fireEvent.change(screen.getByLabelText(/tools for research/i), { target: { value: "Read, Grep" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.tools).toEqual(["Read", "Grep"]);
    fireEvent.change(screen.getByLabelText(/reads for research/i), { target: { value: "src/**" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.reads).toEqual(["src/**"]);
    fireEvent.change(screen.getByLabelText(/writes for research/i), { target: { value: "artifacts/**" } });
    expect(onChange.mock.calls.at(-1)?.[0].teams[0].scope.writes).toEqual(["artifacts/**"]);
  });
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/TeamsStep.test.tsx`
Expected: FAIL — the effort/tools/reads/writes controls don't exist.

- [ ] **Step 3: Implement the extended advanced panel**

In `src/wizard/TeamsStep.tsx`, change the imports line to:

```tsx
import { removeTeam, renameTeam, setTeamModel, setTeamEffort, setTeamTools, setTeamReads, setTeamWrites } from "./draft";
import type { DraftPipeline, EffortMode } from "../ipc/pipeline";
```

Add this helper above the component (maps the `<select>` string value back to an `EffortMode`, preserving the current custom budget when switching to custom):

```tsx
const EFFORT_PRESETS: EffortMode["mode"][] = ["off", "standard", "extended-low", "extended-high", "custom"];

function effortFromSelect(mode: EffortMode["mode"], currentBudget: number): EffortMode {
  return mode === "custom" ? { mode: "custom", budget_tokens: currentBudget } : { mode };
}
```

Replace the advanced-panel block (`openAdvanced === t.id && (...)`, currently only the model input, `:30-42`) with:

```tsx
          {openAdvanced === t.id && (
            <div style={{ marginTop: 6, display: "grid", gap: 6 }}>
              <label style={advLbl}>
                model
                <input
                  aria-label={`model for ${t.id}`}
                  value={t.runner.model}
                  onChange={(e) => onChange(setTeamModel(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
              <label style={advLbl}>
                effort
                <select
                  aria-label={`effort for ${t.id}`}
                  value={t.runner.effort.mode}
                  onChange={(e) =>
                    onChange(
                      setTeamEffort(
                        draft,
                        t.id,
                        effortFromSelect(
                          e.target.value as EffortMode["mode"],
                          t.runner.effort.mode === "custom" ? t.runner.effort.budget_tokens : 16000,
                        ),
                      ),
                    )
                  }
                  style={advInp}
                >
                  {EFFORT_PRESETS.map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              </label>
              {t.runner.effort.mode === "custom" && (
                <label style={advLbl}>
                  budget (tokens)
                  <input
                    type="number"
                    aria-label={`budget for ${t.id}`}
                    value={t.runner.effort.budget_tokens}
                    onChange={(e) => onChange(setTeamEffort(draft, t.id, { mode: "custom", budget_tokens: Number(e.target.value) || 0 }))}
                    style={advInp}
                  />
                </label>
              )}
              <label style={advLbl}>
                tools (comma-separated)
                <input
                  aria-label={`tools for ${t.id}`}
                  value={t.scope.tools.join(", ")}
                  onChange={(e) => onChange(setTeamTools(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
              <label style={advLbl}>
                reads (comma-separated)
                <input
                  aria-label={`reads for ${t.id}`}
                  value={t.scope.reads.join(", ")}
                  onChange={(e) => onChange(setTeamReads(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
              <label style={advLbl}>
                writes (comma-separated)
                <input
                  aria-label={`writes for ${t.id}`}
                  value={t.scope.writes.join(", ")}
                  onChange={(e) => onChange(setTeamWrites(draft, t.id, e.target.value))}
                  style={advInp}
                />
              </label>
            </div>
          )}
```

Add the two shared style constants at the bottom of the file (after the component):

```tsx
const advLbl: React.CSSProperties = { fontSize: 11, color: "var(--text-3)", display: "block" };
const advInp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" };
```

Add `import type React from "react";` at the top if it is not already present.

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/TeamsStep.test.tsx`
Expected: PASS — model still works (existing test) + effort preset + custom budget + tools/reads/writes.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/TeamsStep.tsx src/wizard/TeamsStep.test.tsx
git commit -m "feat(web): per-team advanced panel gains effort/tools/scope (W2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## W3 — Gate authoring in the wizard

> **BLOCKED ON DD1/DD2/DD3 sign-off.** Tasks W3-1..W3-7 implement the recommendation for all three (gates threaded through draft/slice/serialization/best-effort/prompt; manual+chat insertion; kickoff stays teams-only). Task **W3-8** is the UI affordance (DD1 Option C) — **skip W3-8 if the operator rules DD1 Option A**.

### Task W3-1: `DraftPipeline` carries `gates` (+ contract test)

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`
- Modify: `src-tauri/pipeline/src/contract_tests.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    #[test]
    fn empty_draft_has_no_gates() {
        assert!(DraftPipeline::empty().gates.is_empty());
    }

    #[test]
    fn draft_with_gates_round_trips_through_serde_json() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        d.gates.push(Gate { id: "gate-2".into(), label: "Plan review".into(), downstream: "implementers".into() });
        let s = serde_json::to_string(&d).unwrap();
        let back: DraftPipeline = serde_json::from_str(&s).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.gates[0].id, "gate-2");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::tests::empty_draft_has_no_gates`
Expected: FAIL — `no field gates on DraftPipeline`.

- [ ] **Step 3: Add the field**

In `src-tauri/pipeline/src/draft.rs`: add `Gate` to the model import (`:9`):

```rust
use crate::model::{Escalation, Fork, Gate, Join, Pipeline, Routes, RunnerConfig, Scope, Team, Workers, SCHEMA_VERSION};
```

Add the field to `DraftPipeline` (after `joins`, `:69`):

```rust
    #[serde(default)]
    pub gates: Vec<Gate>,
```

Add `gates: vec![],` to `DraftPipeline::empty()` (after `joins: vec![],`, `:89`):

```rust
            joins: vec![],
            gates: vec![],
            escalations: vec![],
```

- [ ] **Step 4: Update the draft key-set contract test**

In `src-tauri/pipeline/src/contract_tests.rs`, update `draft_pipeline_key_set_matches_ts` (`:160`) to include `"gates"`:

```rust
    assert_eq!(
        keys(&v),
        set(&["id", "name", "description", "schema_version", "teams", "gates", "forks", "joins", "escalations"]),
    );
```

- [ ] **Step 5: Run both tests to verify they pass**

Run: `cd src-tauri && cargo test -p pipeline --lib draft:: contract_tests::draft_pipeline_key_set`
Expected: PASS — the draft round-trips with gates; the contract test matches the new key set.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs src-tauri/pipeline/src/contract_tests.rs
git commit -m "feat(pipeline): DraftPipeline carries gates (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-2: `WiringSlice` carries `gates` + `apply_slice` applies them

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    #[test]
    fn wiring_slice_replaces_gates_alongside_forks_and_joins() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        d.teams.push(DraftTeam::new("plan-writers", "Plan Writers"));
        d.teams.push(DraftTeam::new("implementers", "Implementers"));
        apply_slice(&mut d, Slice::Wiring(WiringSlice {
            routes: vec![RouteEdge { team_id: "plan-writers".into(), on_approve: Some("gate-2".into()), on_revise: None, on_reject: None }],
            forks: vec![],
            joins: vec![],
            gates: vec![Gate { id: "gate-2".into(), label: "Plan review".into(), downstream: "implementers".into() }],
        }));
        assert_eq!(d.gates.len(), 1);
        assert_eq!(d.gates[0].downstream, "implementers");
        assert_eq!(d.teams.iter().find(|t| t.id == "plan-writers").unwrap().outputs.on_approve.as_deref(), Some("gate-2"));
    }
```

Also update the EXISTING `wiring_slice_replaces_routes_forks_and_joins` test's `WiringSlice { ... }` literal to add `gates: vec![]` (it now needs the field even though defaulted in serde — the struct literal requires it):

```rust
        apply_slice(&mut d, Slice::Wiring(WiringSlice {
            routes: vec![ /* unchanged */ ],
            forks: vec![Fork { id: "fork-1".into(), lanes: vec!["a".into(), "b".into()] }],
            joins: vec![Join { id: "join-1".into(), waits_for: vec!["a".into(), "b".into()], downstream: "needs-human".into() }],
            gates: vec![],
        }));
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::tests::wiring_slice_replaces_gates`
Expected: FAIL — `no field gates on WiringSlice` (and the existing test won't compile until its literal is updated, which Step 1 already does).

- [ ] **Step 3: Add the field + apply it**

In `WiringSlice` (`:131-138`), add after `joins`:

```rust
    #[serde(default)]
    pub gates: Vec<Gate>,
```

In `apply_slice`'s `Slice::Wiring(s)` arm (`:175-187`), add `draft.gates = s.gates;` alongside the fork/join assignment:

```rust
            draft.forks = s.forks;
            draft.joins = s.joins;
            draft.gates = s.gates;
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: PASS — all draft tests, including the new gate wiring + the updated existing test.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): WiringSlice carries gates; apply_slice applies them (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-3: `to_pipeline` emits the draft's gates

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    #[test]
    fn to_pipeline_emits_the_drafts_gates_and_hard_validates() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        d.id = "demo".into();
        d.name = "Demo".into();
        let mut a = DraftTeam::new("plan-writers", "Plan Writers");
        a.prompt_body = "write the plan".into();
        a.outputs.on_approve = Some("gate-2".into());
        let mut b = DraftTeam::new("implementers", "Implementers");
        b.prompt_body = "implement".into();
        d.teams.push(a);
        d.teams.push(b);
        d.gates.push(Gate { id: "gate-2".into(), label: "Plan review".into(), downstream: "implementers".into() });
        let p = d.to_pipeline();
        assert_eq!(p.gates.len(), 1);
        assert_eq!(p.gates[0].downstream, "implementers");
        // the gate makes implementers reachable -> hard validate passes
        assert_eq!(crate::validate::validate(&p), Ok(()));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::tests::to_pipeline_emits_the_drafts_gates`
Expected: FAIL — `p.gates` is empty (currently hardcoded `gates: vec![]`).

- [ ] **Step 3: Emit the gates**

In `to_pipeline` (`:276`), change `gates: vec![],` to:

```rust
            gates: self.gates.clone(),
```

Update the docstring's "gates are always empty" sentence (`:255`) to: `// each team's inline prompt_body becomes a prompts/<id>.md path; gates carry through (W3).`

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: PASS. Note: the existing `to_pipeline_maps_prompt_body_to_a_prompts_path` test still asserts `p.gates.is_empty()` for a gate-less draft — that still holds (the default draft has no gates). No change needed there.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): to_pipeline emits the draft's gates (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-4: `best_effort_validate` covers gates (downstream + not-inside-a-lane)

**Files:**
- Modify: `src-tauri/pipeline/src/draft.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/draft.rs` `mod tests`:

```rust
    #[test]
    fn best_effort_includes_gates_in_known_nodes() {
        use crate::model::Gate;
        // a team routing to a gate must NOT be flagged as unknown (gates are known nodes)
        let mut d = DraftPipeline::empty();
        let mut a = DraftTeam::new("plan-writers", "Plan Writers");
        a.prompt_body = "x".into();
        a.outputs.on_approve = Some("gate-2".into());
        let mut b = DraftTeam::new("implementers", "Implementers");
        b.prompt_body = "y".into();
        d.teams.push(a);
        d.teams.push(b);
        d.gates.push(Gate { id: "gate-2".into(), label: "G".into(), downstream: "implementers".into() });
        assert_eq!(best_effort_validate(&d), Vec::<String>::new());
    }

    #[test]
    fn best_effort_flags_a_gate_downstream_to_an_unknown_node() {
        use crate::model::Gate;
        let mut d = DraftPipeline::empty();
        let mut a = DraftTeam::new("plan-writers", "Plan Writers");
        a.prompt_body = "x".into();
        d.teams.push(a);
        d.gates.push(Gate { id: "gate-2".into(), label: "G".into(), downstream: "ghost".into() });
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.contains("gate-2") && i.contains("ghost")));
    }

    #[test]
    fn best_effort_flags_a_gate_inside_a_fork_lane() {
        use crate::model::{Fork, Gate, Join};
        // a fork lane team whose on_approve points at a gate (not the join) violates
        // the parallel-flow v1 rule "no gates inside a lane" — surface it live.
        let mut d = DraftPipeline::empty();
        let mut entry = DraftTeam::new("entry", "Entry");
        entry.prompt_body = "x".into();
        entry.outputs.on_approve = Some("fork-1".into());
        let mut la = DraftTeam::new("lane-a", "Lane A");
        la.prompt_body = "x".into();
        la.outputs.on_approve = Some("gate-x".into()); // gate inside the lane
        let mut lb = DraftTeam::new("lane-b", "Lane B");
        lb.prompt_body = "x".into();
        lb.outputs.on_approve = Some("join-1".into());
        let mut after = DraftTeam::new("after", "After");
        after.prompt_body = "x".into();
        d.teams.push(entry);
        d.teams.push(la);
        d.teams.push(lb);
        d.teams.push(after);
        d.forks.push(Fork { id: "fork-1".into(), lanes: vec!["lane-a".into(), "lane-b".into()] });
        d.joins.push(Join { id: "join-1".into(), waits_for: vec!["lane-a".into(), "lane-b".into()], downstream: "after".into() });
        d.gates.push(Gate { id: "gate-x".into(), label: "X".into(), downstream: "join-1".into() });
        let issues = best_effort_validate(&d);
        assert!(issues.iter().any(|i| i.to_lowercase().contains("lane") && i.contains("gate-x")));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::tests::best_effort_flags_a_gate`
Expected: FAIL — gates aren't in `known` (so the gate-route test would falsely flag), gate-downstream isn't checked, gate-in-lane isn't checked.

- [ ] **Step 3: Extend `best_effort_validate`**

In `best_effort_validate` (`:196-244`): add gates to the `known` set (after the joins/escalations inserts, `:207-208`):

```rust
    for g in &draft.gates { known.insert(g.id.as_str()); }
```

After the joins loop (after `:675`/the join `downstream` check), add the gate checks:

```rust
    for g in &draft.gates {
        if !known.contains(g.downstream.as_str()) {
            issues.push(format!("gate '{}' downstream '{}' is unknown", g.id, g.downstream));
        }
    }
    // Parallel-flow v1 rule: no gate may sit inside a fork lane. A lane team whose
    // on_approve points at a gate violates it (hard validate rejects this via
    // LaneNotLinear; surface it live too).
    let mut lane_teams: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for f in &draft.forks {
        for lane in &f.lanes {
            lane_teams.insert(lane.as_str());
        }
    }
    let gate_ids: std::collections::HashSet<&str> = draft.gates.iter().map(|g| g.id.as_str()).collect();
    for t in &draft.teams {
        if lane_teams.contains(t.id.as_str()) {
            if let Some(target) = t.outputs.on_approve.as_deref() {
                if gate_ids.contains(target) {
                    issues.push(format!("team '{}' in a fork lane routes to gate '{}' (no gates inside a lane)", t.id, target));
                }
            }
        }
    }
```

*(Note: the gate-in-lane heuristic flags only the direct case — a lane team's `on_approve` pointing at a gate. Deeper chains are caught by hard validation at create. Best-effort intentionally stays simple; the test above exercises the direct case.)*

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib draft::`
Expected: PASS — all draft tests, including the three new gate best-effort tests and the existing gate-less ones.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/draft.rs
git commit -m "feat(pipeline): best-effort validation covers gates (downstream + no-gate-in-lane) (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-5: Wiring system prompt documents the gate mini-schema (+ a gate-emitting turn test)

**Files:**
- Modify: `src-tauri/pipeline/src/design_session.rs`

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/pipeline/src/design_session.rs` `mod tests`:

```rust
    #[tokio::test]
    async fn wiring_turn_applies_a_gate_from_the_slice() {
        let mut draft = DraftPipeline::empty();
        draft.teams.push(DraftTeam::new("plan-writers", "Plan Writers"));
        draft.teams.push(DraftTeam::new("implementers", "Implementers"));
        let canned = "Adding a human review gate.\n\n```json\n{\"kind\":\"wiring\",\
            \"routes\":[{\"team_id\":\"plan-writers\",\"on_approve\":\"gate-2\",\"on_revise\":null,\"on_reject\":null}],\
            \"forks\":[],\"joins\":[],\
            \"gates\":[{\"id\":\"gate-2\",\"label\":\"Plan review\",\"downstream\":\"implementers\"}]}\n```";
        let runner = FakeChatRunner::new(vec![reply(canned)]);
        let out = design_session_turn(&runner, "sess-1", Step::Wiring, draft, "add a review gate").await;
        assert_eq!(out.updated_draft.gates.len(), 1);
        assert_eq!(out.updated_draft.gates[0].downstream, "implementers");
        assert_eq!(out.updated_draft.teams.iter().find(|t| t.id == "plan-writers").unwrap().outputs.on_approve.as_deref(), Some("gate-2"));
    }

    #[test]
    fn wiring_system_prompt_documents_the_gates_schema() {
        assert!(WIRING_SYSTEM_PROMPT.contains("gates"));
        assert!(WIRING_SYSTEM_PROMPT.contains("downstream"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::tests::wiring_system_prompt`
Expected: FAIL — `WIRING_SYSTEM_PROMPT` has no `gates`. (The `wiring_turn_applies_a_gate_from_the_slice` test will already PASS once W3-2 landed, since `apply_slice` handles gates and the slice parses; running it confirms the end-to-end turn path.)

- [ ] **Step 3: Update the wiring system prompt**

In `src-tauri/pipeline/src/design_session.rs`, replace `WIRING_SYSTEM_PROMPT` (`:101-108`) with:

```rust
const WIRING_SYSTEM_PROMPT: &str = "\
You are wiring the pipeline's flow (routes, optional fork/join lanes, and optional \
human-review gates). Reply with prose, THEN a fenced ```json block, schema:\n\
```json\n{\"kind\":\"wiring\",\
\"routes\":[{\"team_id\":\"<id>\",\"on_approve\":\"<id|null>\",\"on_revise\":null,\"on_reject\":null}],\
\"forks\":[{\"id\":\"fork-1\",\"lanes\":[\"<team id>\",\"<team id>\"]}],\
\"joins\":[{\"id\":\"join-1\",\"waits_for\":[\"<team id>\",\"<team id>\"],\"downstream\":\"<id>\"}],\
\"gates\":[{\"id\":\"gate-1\",\"label\":\"<human-readable>\",\"downstream\":\"<id>\"}]}\n```\n\
A gate is a human-review checkpoint: a team routes to it via on_approve, and the \
gate forwards approved work to its downstream. A fork must have >=2 lanes; NEVER \
place a gate inside a fork lane. routes stay single-target. Only the json mutates state.";
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p pipeline --lib design_session::`
Expected: PASS — both new tests + all existing design_session tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/pipeline/src/design_session.rs
git commit -m "feat(pipeline): wiring system prompt documents the gate schema + no-gate-in-lane rule (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-6: TS `DraftPipeline` carries `gates` + `emptyDraft` seeds it

**Files:**
- Modify: `src/ipc/pipeline.ts`
- Modify: `src/wizard/draft.ts`

- [ ] **Step 1: Write the failing test**

Add to `src/wizard/draft.test.ts`:

```ts
it("emptyDraft seeds an empty gates array", () => {
  expect(emptyDraft().gates).toEqual([]);
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: FAIL — `emptyDraft().gates` is `undefined`.

- [ ] **Step 3: Add `gates` to the TS type + `emptyDraft`**

In `src/ipc/pipeline.ts`, add `gates: Gate[];` to the `DraftPipeline` interface (after `joins`, `:104`):

```ts
export interface DraftPipeline {
  id: string;
  name: string;
  description: string;
  schema_version: number;
  teams: DraftTeam[];
  forks: Fork[];
  joins: Join[];
  gates: Gate[];
  escalations: Escalation[];
}
```

In `src/wizard/draft.ts`, add `gates: [],` to `emptyDraft()` (after `joins: [],`, `:16`):

```ts
    forks: [],
    joins: [],
    gates: [],
    escalations: [],
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/pipeline.ts src/wizard/draft.ts
git commit -m "feat(web): DraftPipeline carries gates; emptyDraft seeds them (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-7: `draftToPipeline` adapter emits gates (Wiring + Review render them)

**Files:**
- Modify: `src/wizard/WiringStep.tsx`
- Modify: `src/wizard/WiringStep.test.tsx`

- [ ] **Step 1: Write the failing test**

Add to `src/wizard/WiringStep.test.tsx`:

```ts
  it("renders gates via PipelineView", () => {
    let d = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");
    d = { ...d, gates: [{ id: "gate-2", label: "Plan review", downstream: "implementers" }] };
    render(<WiringStep draft={d} onChange={() => {}} />);
    expect(screen.getByText(/Gates \(1\)/)).toBeInTheDocument();
    expect(screen.getByText(/Plan review/)).toBeInTheDocument();
  });
```

*(Note the new `onChange` prop — Task W3-8 adds the add-gate affordance which needs it. If DD1 Option A is chosen and W3-8 is skipped, `WiringStep` keeps no `onChange` prop; in that case drop `onChange={() => {}}` here and from `NewProjectWizard`.)*

- [ ] **Step 2: Run the test to verify it fails**

Run: `npm test -- src/wizard/WiringStep.test.tsx`
Expected: FAIL — `Gates (1)` not rendered (the adapter hardcodes `gates: []`).

- [ ] **Step 3: Emit gates in the adapter**

In `src/wizard/WiringStep.tsx`, change `draftToPipeline` (`:13`) `gates: [],` to:

```ts
    gates: d.gates,
```

Update the docstring (`:5`): `inline prompt bodies become placeholder paths; gates carry through (W3).`

- [ ] **Step 4: Run the test to verify it passes**

Run: `npm test -- src/wizard/WiringStep.test.tsx`
Expected: PASS — gates render through `PipelineView`'s existing "Gates (N)" section. The Review step renders the same adapter, so gates appear there too automatically.

- [ ] **Step 5: Commit**

```bash
git add src/wizard/WiringStep.tsx src/wizard/WiringStep.test.tsx
git commit -m "feat(web): draftToPipeline emits gates so Wiring + Review render them (W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task W3-8: Add-gate affordance in the Wiring step (DD1 Option C — SKIP if operator rules Option A)

**Files:**
- Modify: `src/wizard/draft.ts`
- Modify: `src/wizard/draft.test.ts`
- Modify: `src/wizard/WiringStep.tsx`
- Modify: `src/wizard/WiringStep.test.tsx`
- Modify: `src/wizard/NewProjectWizard.tsx`

- [ ] **Step 1: Write the failing draft-helper test**

Add to `src/wizard/draft.test.ts`:

```ts
import { addGate, removeGate, setTeamApprove } from "./draft";

describe("gate helpers (W3)", () => {
  const base = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");

  it("addGate appends a gate node", () => {
    const d = addGate(base, "gate-2", "Plan review", "implementers");
    expect(d.gates).toEqual([{ id: "gate-2", label: "Plan review", downstream: "implementers" }]);
  });

  it("addGate is a no-op on a duplicate id", () => {
    const d = addGate(addGate(base, "gate-2", "Plan review", "implementers"), "gate-2", "again", "implementers");
    expect(d.gates).toHaveLength(1);
  });

  it("setTeamApprove repoints a team's on_approve", () => {
    const d = setTeamApprove(base, "plan-writers", "gate-2");
    expect(d.teams.find((t) => t.id === "plan-writers")?.outputs.on_approve).toBe("gate-2");
  });

  it("removeGate drops the gate node", () => {
    const d = removeGate(addGate(base, "gate-2", "Plan review", "implementers"), "gate-2");
    expect(d.gates).toHaveLength(0);
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: FAIL — `addGate`/`removeGate`/`setTeamApprove` not exported.

- [ ] **Step 3: Implement the gate helpers**

In `src/wizard/draft.ts`, add (import `Gate` in the top type import: `import type { DraftPipeline, DraftTeam, EffortMode, Gate } from "../ipc/pipeline";`):

```ts
export function addGate(d: DraftPipeline, id: string, label: string, downstream: string): DraftPipeline {
  if (d.gates.some((g) => g.id === id)) return d;
  const gate: Gate = { id, label, downstream };
  return { ...d, gates: [...d.gates, gate] };
}

export function removeGate(d: DraftPipeline, id: string): DraftPipeline {
  return { ...d, gates: d.gates.filter((g) => g.id !== id) };
}

export function setTeamApprove(d: DraftPipeline, teamId: string, target: string | null): DraftPipeline {
  return mapTeams(d, (t) => (t.id === teamId ? { ...t, outputs: { ...t.outputs, on_approve: target } } : t));
}
```

- [ ] **Step 4: Run to verify the helpers pass**

Run: `npm test -- src/wizard/draft.test.ts`
Expected: PASS.

- [ ] **Step 5: Write the failing UI test**

Add to `src/wizard/WiringStep.test.tsx`:

```ts
  it("the add-gate control adds a gate and repoints the upstream team's on_approve", () => {
    const d = addTeam(addTeam(emptyDraft(), "plan-writers", "Plan Writers"), "implementers", "Implementers");
    const onChange = vi.fn();
    render(<WiringStep draft={d} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/gate id/i), { target: { value: "gate-2" } });
    fireEvent.change(screen.getByLabelText(/gate label/i), { target: { value: "Plan review" } });
    fireEvent.change(screen.getByLabelText(/gate upstream/i), { target: { value: "plan-writers" } });
    fireEvent.change(screen.getByLabelText(/gate downstream/i), { target: { value: "implementers" } });
    fireEvent.click(screen.getByRole("button", { name: /add gate/i }));
    const next = onChange.mock.calls.at(-1)?.[0];
    expect(next.gates).toEqual([{ id: "gate-2", label: "Plan review", downstream: "implementers" }]);
    expect(next.teams.find((t: { id: string }) => t.id === "plan-writers").outputs.on_approve).toBe("gate-2");
  });
```

Update the test file's imports to include `fireEvent` and `vi` (the existing file imports `render, screen`):

```ts
import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
```

- [ ] **Step 6: Run to verify the UI test fails**

Run: `npm test -- src/wizard/WiringStep.test.tsx`
Expected: FAIL — no add-gate control; `WiringStep` has no `onChange` prop.

- [ ] **Step 7: Implement the add-gate affordance**

Replace `src/wizard/WiringStep.tsx` with (adds an `onChange` prop + a small "Add human gate" form above the read-only `PipelineView`; selecting an upstream team and a downstream node, then "Add gate", composes `addGate` + `setTeamApprove`):

```tsx
import { useState } from "react";
import type { DraftPipeline, Pipeline } from "../ipc/pipeline";
import { PipelineView } from "../components/PipelineView";
import { addGate, setTeamApprove } from "./draft";

/// Adapt a DraftPipeline to the Pipeline shape the read-only viewer expects:
/// inline prompt bodies become placeholder paths; gates carry through (W3).
export function draftToPipeline(d: DraftPipeline): Pipeline {
  return {
    id: d.id || "(draft)",
    name: d.name || "(unnamed)",
    description: d.description,
    schema_version: d.schema_version,
    teams: d.teams.map((t) => ({ ...t, prompt: `prompts/${t.id}.md` })),
    gates: d.gates,
    escalations: d.escalations,
    forks: d.forks,
    joins: d.joins,
  };
}

interface WiringStepProps {
  draft: DraftPipeline;
  onChange: (d: DraftPipeline) => void;
}

/// Step 4 draft view: the fork/join/gate flow rendered through PipelineView, plus
/// an add-human-gate affordance (DD1 Option C). Adding a gate = push the gate node
/// AND repoint the chosen upstream team's on_approve at it (DD2: upstream implied).
export function WiringStep({ draft, onChange }: WiringStepProps) {
  const [gid, setGid] = useState("");
  const [label, setLabel] = useState("");
  const [upstream, setUpstream] = useState("");
  const [downstream, setDownstream] = useState("");

  const canAdd = gid.trim() && label.trim() && upstream && downstream;
  const nodeOptions = [
    ...draft.teams.map((t) => ({ id: t.id, name: t.name })),
    ...draft.escalations.map((e) => ({ id: e.id, name: e.id })),
    ...draft.joins.map((j) => ({ id: j.id, name: j.id })),
  ];

  function add() {
    if (!canAdd) return;
    let next = addGate(draft, gid.trim(), label.trim(), downstream);
    next = setTeamApprove(next, upstream, gid.trim());
    onChange(next);
    setGid("");
    setLabel("");
    setUpstream("");
    setDownstream("");
  }

  return (
    <div>
      <div style={{ display: "grid", gap: 6, marginBottom: "var(--sp-3)", padding: "var(--sp-2)", border: "1px solid var(--border)", borderRadius: "var(--r-sm)" }}>
        <div style={{ fontSize: 11, color: "var(--text-3)" }}>Add a human-review gate</div>
        <input aria-label="gate id" placeholder="gate id (e.g. gate-2)" value={gid} onChange={(e) => setGid(e.target.value)} style={inp} />
        <input aria-label="gate label" placeholder="label (e.g. Plan review)" value={label} onChange={(e) => setLabel(e.target.value)} style={inp} />
        <select aria-label="gate upstream" value={upstream} onChange={(e) => setUpstream(e.target.value)} style={inp}>
          <option value="">upstream team (its on_approve routes here)…</option>
          {draft.teams.map((t) => (
            <option key={t.id} value={t.id}>{t.name} ({t.id})</option>
          ))}
        </select>
        <select aria-label="gate downstream" value={downstream} onChange={(e) => setDownstream(e.target.value)} style={inp}>
          <option value="">downstream node…</option>
          {nodeOptions.map((n) => (
            <option key={n.id} value={n.id}>{n.name} ({n.id})</option>
          ))}
        </select>
        <button onClick={add} disabled={!canAdd} aria-label="add gate">Add gate</button>
      </div>
      <PipelineView pipeline={draftToPipeline(draft)} />
    </div>
  );
}

const inp: React.CSSProperties = { width: "100%", background: "var(--bg-2)", border: "1px solid var(--border)", color: "var(--text)", padding: "var(--sp-2)", borderRadius: "var(--r-sm)" };
```

Add `import type React from "react";` at the top if not present.

- [ ] **Step 8: Pass `onChange` from `NewProjectWizard`**

In `src/wizard/NewProjectWizard.tsx`, update the wiring `renderDraft` (`:81`) to pass `onChange`:

```tsx
                renderDraft={(d, onChange) => <WiringStep draft={d} onChange={onChange} />}
```

- [ ] **Step 9: Run the UI test + the wizard nav test**

Run: `npm test -- src/wizard/WiringStep.test.tsx src/wizard/NewProjectWizard.test.tsx`
Expected: PASS — add-gate composes the gate + the route; the wizard still navigates (the `onChange` from `ChatDraftPanel`'s `handleManualEdit` flows to `WiringStep`, so a manual gate add also re-fetches best-effort issues, tying W1+W3 together).

- [ ] **Step 10: Commit**

```bash
git add src/wizard/draft.ts src/wizard/draft.test.ts src/wizard/WiringStep.tsx src/wizard/WiringStep.test.tsx src/wizard/NewProjectWizard.tsx
git commit -m "feat(web): add-human-gate affordance in the Wiring step (W3, DD1 Option C)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task FINAL: Full-suite green gate + DOMAIN.md note

**Files:**
- Modify: `DOMAIN.md`

- [ ] **Step 1: Run the FULL backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS — every crate green (pipeline incl. draft/design_session/contract_tests, app incl. design_session_tests, all others untouched).

- [ ] **Step 2: Run the FULL frontend suite**

Run: `npm test`
Expected: PASS — every test green (wizard/*, ipc/*, App, components).

- [ ] **Step 3: Verify the dependency graph is still acyclic**

Run: `cd src-tauri && cargo tree -p pipeline -i llm_chat`
Expected: `pipeline` depends on `llm_chat`; no cycle (W1/W2/W3 add no new crate edges).

- [ ] **Step 4: Update DOMAIN.md**

In `DOMAIN.md`, under `### Pipeline Authoring`, append to the **DraftPipeline** line (or add a sentence): the wizard now surfaces **best-effort validation** live during steps 2–4 (W1), supports a full **per-team advanced panel** (model, effort preset, tools, scope reads/writes — W2), and authors **human-review gates** in the Wiring step (`DraftPipeline` carries gates; `to_pipeline` emits them; the no-gate-inside-a-lane parallel-flow rule is honored — W3).

- [ ] **Step 5: Commit**

```bash
git add DOMAIN.md
git commit -m "docs(domain): wizard polish — live best-effort, full advanced panel, gate authoring (W1/W2/W3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Roadmap

After this lands, the brainstorming wizard's three deferred polish items (W1/W2/W3 from `docs/v1.1-backlog.md`) are complete. Remaining wizard-adjacent backlog (not in scope here): templates returning as wizard *seeds* (A2), token streaming for the Design Session chat (C2), and pipeline editor write-mode for existing graphs (A1). W3 deliberately keeps gates as a *flat* affordance honoring the parallel-flow v1 rule (no gates inside a lane); nested groups (P1) would generalize that and are tracked separately.

---

## Self-review (run against the brief)

- **W1 coverage:** TurnResult.issues returned from the value the backend already computes (W1-1); a backend `best_effort_validate_cmd` for manual edits so no validation logic enters the frontend (W1-2); TS mirror + wrapper + wire-shape lock (W1-3); the `ChatDraftPanel` banner renders turn issues AND re-fetches on manual edit (W1-4). Matches the brief's "prefer surfacing the existing best-effort result; add issues to the turn result if absent" — it WAS absent, so it's added.
- **W2 coverage:** effort preset select (all five `EffortMode` presets incl. custom budget), tools editor, reads/writes scope editors — same controlled-input + `mapTeams` pattern as `setTeamModel` (W2-1, W2-2). Reuses `EffortMode`/`Scope` shapes verbatim.
- **W3 coverage:** `DraftPipeline.gates` (W3-1, + contract test), `WiringSlice.gates` + `apply_slice` (W3-2), `to_pipeline` emits gates (W3-3), best-effort covers gate-downstream + no-gate-in-lane (W3-4), wiring system prompt (W3-5), TS type + `emptyDraft` (W3-6), `draftToPipeline` adapter so Wiring+Review render gates (W3-7), and the add-gate UI affordance (W3-8). `validate.rs`/`route()`/Runtime untouched (gates already validated + routed). DD1/DD2/DD3 surfaced at the top for sign-off; W3-8 is explicitly skippable for DD1 Option A.
- **No live `claude`:** every backend test uses `FakeChatRunner` with canned replies (W1-1, W3-5); every frontend test mocks the IPC. No test spawns `claude`.
- **Type consistency:** `issues` is named identically across `TurnResult` (Rust + TS); `bestEffortValidate`/`best_effort_validate_cmd` paired; `gates`/`Gate` used identically across `DraftPipeline` (Rust+TS), `WiringSlice`, `apply_slice`, `to_pipeline`, `draftToPipeline`; `setTeamEffort`/`setTeamTools`/`setTeamReads`/`setTeamWrites`/`addGate`/`removeGate`/`setTeamApprove` defined once each in `draft.ts` and used in `TeamsStep`/`WiringStep`.
- **Placeholder scan:** every code step shows the full code; every test step shows the assertion + the run command + expected output; every task ends in a commit.
