---
id: 2026-06-24-vet-graph-builder-language
target: interactive node graph builder (Pipeline Authoring) — UI vocabulary
date: 2026-06-24
verb: vet
mode: vet · strategic · workshop
operator: tim.healey@splose.com
sources:
  - DOMAIN.md
  - docs/context-map.md
  - src-tauri/pipeline/src/model.rs
  - src/lib/pipelineGraph.ts
  - docs/v1.1-backlog.md (L1)
---

# Vet — Interactive node graph builder: language consolidation

A pre-build language vet of the proposed **interactive node graph builder**: a
React-Flow canvas that becomes the primary Pipeline Authoring surface (replacing
the Teams/Prompts/Wiring wizard form steps and powering edit-mode), with
`DraftPipeline` as the single source of truth, a node palette, edges = `Routes`,
and a per-node editor exposing the full team config. Findings trace to
`signals.md` §E (design-stage smells). Goal: the graph is intuitive *with* the
domain model, not a parallel vocabulary.

## Verdict

**Sound, with language consolidations.** The architecture (DraftPipeline as
single source of truth, canvas as derived view, no backend change) keeps the
Pipeline Authoring ↔ Runtime boundary clean. The substantive findings are
language: one Runtime concept leaking into the authoring surface (**F2**), one
overloaded route field that the operator correctly spotted is *not* a verdict
for producer roles (**F3**), and the unnamed **role** concept underneath both
F3 and L1 (**F8**). Node-kind extensibility (**F9**) is the forward-compat hook
for the in-flight data-store model work.

---

### F1 [low] off-language naming — node editor chrome
**What.** The per-node editor was provisionally called "Inspector." Generic
IDE furniture, not a domain term.
**Cited.** Design proposal (canvas-primary + inspector mock, screen 06).
**Why it matters.** Generic chrome (canvas, palette, panel) needs no
ubiquitous-language name — the risk is only if "Inspector" starts carrying
domain meaning. The concept being edited is a **Node** (`NodeKind` =
`Team | Gate | Escalation | Fork | Join`, `model.rs:208-216`), not specifically
a Team (see F9).
**Amendment.** Call it the **node drawer** (matching the app's existing
`CardDrawer`), with a header that names the selected node's kind + id ("Team ·
spec-writers", "Gate · spec gate"). Generic container, kind-specific contents.
**Status:** resolved (node drawer)

### F2 [high] cross-boundary concept + off-language — "Workers (default/max)"
**What.** `Team.workers: Workers { default, max }` (`model.rs:60,149-159`)
surfaced in the editor as "Workers." "Workers" as live processes is a **Runtime**
concept — the `WorkerPool` aggregate owns `workers[]` (pid, current_task) with
invariant `workers.count ≤ team.workers.max` (`context-map.md:110-117`). The
operator's word for this authoring dial is **"scale"** (`DOMAIN.md` Operator
vocab).
**Cited.** `model.rs:149-159`; `context-map.md:116`; `DOMAIN.md` Operator vocab.
**Why it matters.** Labeling the authored *capacity policy* "Workers" leaks
Runtime's live-process noun into the authoring surface (§E cross-boundary). It
also exposed a **code/doc drift**: the struct field is `default`, but the
context-map invariant calls the same value `min` (`workers.count ≥ team.workers.min`).
**Amendment.** (a) Editor control titled **"Scale (min · max)"** — never
"Workers." (b) Rename the Rust field **`default → min`** to align code with the
context-map, shipped as its **own `remediate`**, not folded into the graph
builder.
**Status:** resolved — UI label "Scale"; field rename queued as a separate remediate

### F3 [medium] off-language / overloaded route — producer edges are not verdicts
**What.** Initial recommendation was to label all edges `approve/revise/reject`.
The operator correctly objected: the edge from a **producer** (e.g. spec-writers)
to its reviewer is a hand-off, not a verdict. `Routes.on_approve`
(`model.rs:139-147`) is **overloaded** — a *Verdict* (a Review/Runtime concept;
`DOMAIN.md` *Verdict*) when the source is a reviewer/gate, a *hand-off* when the
source is a producer.
**Cited.** `model.rs:139-147`; `DOMAIN.md` Route/Verdict; `lib/pipelineGraph.ts`
(`EdgeKind = "forward" | "revise" | "escalate"` — already off-language:
"escalate" is a node kind, not a route).
**Why it matters.** Imposing verdict vocabulary on non-verdict roles is the
**same root as L1's DOA bug** — `parse_verdict` defaults producers to `revise`
(`v1.1-backlog.md` L1). The name lies for the producer role.
**Amendment.** Edge labels are **role-dependent**: a **reviewer/gate** source
shows `approve · revise · reject`; a **producer** source shows a single
**hand-off** edge. "escalate"/"forward" retired as edge words (the edge into an
escalation node is still `reject`). The read-only `PipelineGraph` should adopt
the same vocabulary (refactor-before-add: reshape the shared edge vocabulary,
don't duplicate the off-language one).
**Status:** resolved (role-dependent labels; producer = "hand-off")

### F4 [low] confirm precise parallel-flow terms — Fork/Join drawer
**What.** Fork/Join editing must use existing hard-won language, not generic
"branches/parallelism."
**Cited.** `model.rs:177-205`; `DOMAIN.md` (Lane / Quorum / Early-cancel).
**Why it matters.** **Lane** was deliberately chosen over "branch" to avoid the
git collision (`DOMAIN.md`); **Quorum** (N-of-M) and **Early-cancel**
(`cancel_on_reject`) are settled terms.
**Amendment.** Join drawer fields: **"Waits for lanes," "Quorum (N of M),"
"Early-cancel on reject."** This is where the deferred P2/P3 authoring controls
finally land. Never "branch."
**Status:** resolved (adopt existing terms)

### F5 [low · affirmation] DraftPipeline ≠ Pipeline holds
**What.** The canvas edits a **DraftPipeline**; only hard validation at
create/save mints a **Pipeline**.
**Cited.** `DOMAIN.md` (DraftPipeline vs Pipeline); A1 `from_pipeline`.
**Why it matters.** Keeping the draft as the single source of truth (best-effort
issues non-blocking; edit-mode loads via `from_pipeline`→draft) preserves the
boundary — *provided* the canvas reuses the existing draft machinery rather than
inventing parallel state.
**Status:** resolved (affirmed; reuse draft machinery)

### F6 [low · guardrail] keep the LLM touchpoint sealed
**What.** "Generate recommended graph" routes through the Design Session (LLM
Chat ACL). The canvas itself must never call the model.
**Cited.** `DOMAIN.md` (LLM Chat ACL / Design Session); `ChatDraftPanel`.
**Amendment.** Only LLM touchpoint is the Basics "Generate." The deferred
command bar, when it returns, sits behind the existing `ChatDraftPanel`/`llm_chat`
seam — not on the canvas directly.
**Status:** resolved (sealed; command bar deferred)

### F7 [low · guardrail] `api_key_env` is a name, not a secret
**What.** "Full team config" includes `runner.api_key_env`.
**Cited.** `model.rs:71-72,107`; S1 secrets crate / no-secret-egress.
**Why it matters.** `api_key_env` is an env-var *name*; the actual key lives in
the keychain and never crosses the OHS. Safe to show.
**Amendment.** Drawer surfaces the var name only, never the key.
**Status:** resolved (name only)

### F8 [high] unnamed concept — "role"
**What.** Producer vs reviewer is not modeled — it is inferred from the team
name by a regex (`teamRole`, `lib/pipelineGraph.ts`).
**Cited.** `lib/pipelineGraph.ts` `teamRole`; F3 above; `v1.1-backlog.md` L1.
**Why it matters.** The verdict-vs-hand-off edge distinction (F3) and L1's
"producers default approve, reviewers judge" both require this concept named.
Inferring edge semantics from a name regex is brittle.
**Agreed contract.** `Team.role: producer | reviewer` — additive enum,
`#[serde(default)] = producer` (matches L1). **Reviewer** emits the verdict
triple; **producer** (incl. implementers) emits a single hand-off edge. Visual
coloring (writer/impl/reviewer) stays a separate display concern.
**Amendment / routing.** **Defined now, implemented in the model/L1 session**
(not the graph builder — it is a shared-aggregate change that would collide with
the in-flight model work). The graph builder reads `Team.role` when present and
**falls back to the `teamRole` regex until the field exists**.
**Status:** deferred (contract pinned here; field implemented in model/L1 session)

### F9 [medium] node-kind extensibility — forward-compat with data-store nodes
**What.** `NodeKind` is a closed set today (`Team | Gate | Escalation | Fork |
Join`, `model.rs:208-216`), but the in-flight model work adds kinds (data stores
/ typed containers — the data-plane idea).
**Cited.** `model.rs:208-216`; this session's data-node orientation.
**Why it matters.** A canvas hard-wired to the current five kinds would need a
rewrite when a new kind lands. "Node ⊋ Team" (operator) is the design cue.
**Amendment.** Palette, node drawer, and `draftToFlow` are **driven by the
`NodeKind` set** — a new kind slots in as one palette entry + one drawer form +
one renderer, not a restructure. Realizes the "build now, stay forward-compatible
with the model work" posture.
**Status:** resolved (extensibility as a design constraint)

---

## Consolidated labels (canonical)

| Surface | Model field | ❌ avoid | ✅ label |
|---|---|---|---|
| Palette | `NodeKind` | — | **Team · Gate · Fork · Join · Escalation** (+ future kinds, F9) |
| Edge — from reviewer/gate | `Routes.on_approve/revise/reject` | forward/escalate/link | **approve · revise · reject** |
| Edge — from producer | `Routes.on_approve` | approve | **hand-off** |
| Node editor | (chrome) | Inspector | **node drawer** (header: kind · id) |
| Team | `runner.*` | — | **Runner** (kind · model · effort · API-key env var) |
| Team | `scope.*` | permissions | **Scope** (reads · writes · tools) |
| Team | `workers.{min,max}` | Workers | **Scale** (min · max) |
| Join | `waits_for/quorum/cancel_on_reject` | branches/parallelism | **Lanes** / **Quorum (N of M)** / **Early-cancel on reject** |

## Hand-offs out of this vet

- **F2 field rename** `default → min` — separate `remediate` (model touch).
- **F8 `Team.role`** — defined here (`producer | reviewer`, default producer);
  implemented in the model/L1 session. Graph builder infers best-effort until then.
