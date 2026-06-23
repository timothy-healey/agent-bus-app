---
id: vet-b1-comment-reanchoring-2026-06-23
target: plans/2026-06-23-plan-b1-comment-reanchoring.md
date: 2026-06-23
mode: vet (pre-build DDD gate)
lens: strategic · vet · brief
operator: tim.healey@splose.com
verb: ddd-council vet
---

# Vet — B1 Comment re-anchoring across artifact versions

**Verdict: SOUND WITH FIXES.** The design places re-anchoring in the context that owns the concepts (Review owns `Artifact`, `Comment`, `Thread`), the `<!-- addressed: <comment-id> -->` marker is registered as Review ubiquitous language, the projection is read-derived (no schema/persistence change), and the Review↔Runtime **Conformist** seam is untouched (the new command reads comments and parses markdown — it emits no verdict and mutates no Task state). No boundary bypass. B1's re-anchoring is correctly distinct from B2's display-only line-diff. Three findings, all low/medium and all language/clarity — every recommended fix is applied to the plan in this pass.

## Room

- **Architect** — "Where does re-anchoring live, and does any seam move?" Review already owns Comment/Artifact/Thread; the projection reads its own aggregate and parses an artifact body Review already renders. No new edge to Runtime, Runners, or Telemetry. The OHS tool list grows by one read tool under `supplier_context: "review"` — same surface the terminal already consumes. Seam holds.
- **Engineer** — "Refactor vs add, and is the Conformist seam really clean?" The parser/projector are genuinely new pure functions (no existing code does marker parsing), so the addition is justified — but the frontend `useComments` fallback hand-builds a `ReanchoredComment` (status `open`, offset = stored), which **duplicates** the Rust projector's no-marker branch. That's a §E *adds-where-a-refactor-fits* whiff in miniature: two encodings of the same default. Acceptable (the fallback exists for callers that pass no version markdown, and round-tripping every list through the backend command would be chattier), but it must be documented as mirroring the projector so the two don't drift. The seam is clean: `reanchor_comments` calls only `list_for_task` + the pure projector, returns a read projection, and records nothing — no `record_verdict`, no `*_gate`.
- **Engineer (Review) + Operator** — "Is `addressed` the right word?" DOMAIN's `Thread` is defined as "a comment + its **resolutions** across versions". An `addressed` marker is precisely one such resolution event, and `Comment status = open | addressed` is the per-version read of the Thread. The language coheres — but the plan introduces `effective_offset` and `Re-anchoring` as terms that DOMAIN does not yet carry. Register them so the names in the code are the names in the language.
- **AI Engineer** — "Does the marker convention leak Claude's idiom?" No — the marker is our convention written *into* the artifact a writer produces; it is parsed by Review, never crosses the Runners ACL as a Claude protocol token. Fine.

Converged: sound; apply the three language/clarity fixes below.

---

## Findings

### F1 [low] off-language-naming — `effective_offset` / `Re-anchoring` not in the ubiquitous language

**What.** The plan introduces `ReanchoredComment.effective_offset`, `CommentStatus = open | addressed`, and the verb "re-anchor" as load-bearing names, but `DOMAIN.md` → Review carries none of them.
**Cited.** Plan §Decisions DD4/DD5, Task 2 (`ReanchoredComment`, `effective_offset`), Task 9. DOMAIN.md `### Review` (only `Artifact`/`Comment`/`Thread`/`Send back`).
**Why it matters.** §E off-language-naming: if the term isn't in DOMAIN, the code puts language in front of the model the room never ratified. These names are good — they just need to be canon.
**Suggested amendment.** Task 9 already adds **Addressed marker**, **Re-anchoring**, and **Comment status** to DOMAIN Review. Extend Task 9 to also name the **re-anchored (effective) offset** explicitly so `effective_offset` traces to a domain term. (Applied — see Task 9 edit below.)
**Status:** resolved

### F2 [low] contradicts-domain (refines) — re-anchoring should be tied to the existing `Thread` concept, not introduced beside it

**What.** The plan presents re-anchoring as a fresh projection without connecting it to `Thread` ("a comment + its resolutions across versions"), which already names exactly this idea. Risk: a reader treats `addressed`/`open` as a new parallel concept rather than the per-version read of a Thread's resolution history.
**Cited.** Plan §Architecture + DD4; DOMAIN.md `### Review` `**Thread**`.
**Why it matters.** §E contradicts-`DOMAIN.md`: not a conflict, but an un-anchored term that should explicitly realize a settled one. Naming the link keeps one model in the context.
**Suggested amendment.** In the Task 9 DOMAIN edit, state that re-anchoring is how a **Thread**'s "resolutions across versions" are computed for a viewed version (an `addressed` marker is a resolution; `open`/`addressed` is the Thread's per-version status). (Applied — Task 9 edit references Thread.)
**Status:** resolved

### F3 [medium] adds-where-a-refactor-fits — the frontend carry-over fallback duplicates the projector's no-marker branch

**What.** `useComments` (Task 6) synthesizes `ReanchoredComment` objects (`status: "open"`, `effective_offset: c.anchor_offset`) in TypeScript when no version markdown is supplied — a second encoding of the Rust projector's `(None, _) => Open at stored offset` rule. Two sources of truth for the same default invite drift.
**Cited.** Plan Task 6 Step 3 (the `setReanchored(list.map(...))` fallback) vs Task 2 `reanchor_comments` no-marker branch.
**Why it matters.** §E adds-where-a-refactor-fits / §C one-concept-two-encodings. The default-projection rule (no marker ⇒ open, stored offset) lives in two languages.
**Suggested amendment.** Keep the fallback (it avoids a backend round-trip for callers with no version body and preserves v1 carry-over back-compat), but add a doc-comment on the fallback branch stating it deliberately mirrors the Rust projector's no-marker default and must change in lockstep with it; and have the projector's doc-comment note the same. The single authoritative path remains the Rust projector whenever a version body is present (the only path that ever produces `addressed`). (Applied — Task 6 fallback gains the mirroring doc-comment; Task 2 projector doc-comment cross-references it.)
**Status:** resolved

---

## Boundary & seam summary (for the record)

- **Ownership:** re-anchoring, the marker convention, and `CommentStatus` all sit in **Review** — the context that owns `Comment`/`Artifact`/`Thread`. ✓
- **Conformist seam (Review→Runtime):** untouched. `reanchor_comments` is read-only; no verdict emitted, no Task state mutated. ✓
- **OHS:** one new read tool (`reanchor_comments`) under `supplier_context: "review"`; same host surface the terminal already consumes. ✓
- **No boundary bypass / no accidental shared kernel:** the projection reads only Review's own `comments` table + an artifact body (already a Review input); introduces no type other contexts depend on. ✓
- **Distinct from B2:** B2's `lineDiff` is display-only line identity; B1 re-anchors comment identity across versions. Documented in DOMAIN + the projector doc-comment. ✓
- **Persistence:** read-derived; no migration, no `SCHEMA_VERSION` bump. ✓
