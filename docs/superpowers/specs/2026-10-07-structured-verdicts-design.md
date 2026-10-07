# Spec — Structured verdicts

*Design doc. Wayfinder map: "Map: runner capabilities from the current Claude CLI" (#3). Ticket: "Spec: structured verdicts" (#8). Research: "Research: --json-schema under --print stream-json" (#7, `docs/research/json-schema-print-stream.md` on `research/json-schema-print`). Each worker's verdict, artifact path and generator items come back as schema-validated **Structured output** instead of text markers. A reviewer's verdict carries a **reason** that reaches the producer, and the engine acts on reviewer verdicts on every edge. Terms are as defined in `DOMAIN.md` (Runners, Runtime).*

## Why this exists

Workers report their results through text markers (`KEY:`, `DESCRIPTION:`, `ARTIFACT:`, `VERDICT:`) that `runners/src/stream_json.rs` parses leniently from assistant prose. That causes four problems:

- **Wrong defaults.** A missing or garbled marker is a guess. The parser defaults a missing verdict to Revise, while the engine defaults it to Approve (`engine.rs:415`, `:1438`). In live runs, research output without markers was read as revise and escalated straight to needs-human (`docs/v1.1-backlog.md:36`).
- **No reason.** A reviewer's verdict carries no reason, so a revise reaches the producer with nothing to act on.
- **Ignored verdicts.** The engine only reads a reviewer's verdict when its downstream is a gate or a join. On a plain team→team edge the item always goes forward; `on_revise` / `on_reject` are never read.
- **Lost comments.** The revision bundle looks up comments by task id. A revise creates a new child task, so feedback attached to the parent can be lost.

The CLI can validate output itself. `--json-schema <schema>` adds a `StructuredOutput` tool, the CLI validates the model's call against the schema, and the validated object arrives as `result.structured_output`. It works with tools under `--print --output-format stream-json`. If the model can't satisfy the schema, the CLI nudges it once (`[structured-output-enforce]`). If that fails too, the run still ends `subtype: success`, exit 0, with prose in `result` and **no `structured_output`**: that absence is the only failure signal. The schema overrides any `VERDICT:` instruction in the system prompt, so the switch must be total. Verified on `claude` 2.1.289.

## Decisions

1. **Every worker invocation passes `--json-schema`**, with one schema per stage kind. `result.structured_output` is the only source of a worker's items, verdict and artifact. The marker parsers (`parse_verdict`, `parse_artifact`, `parse_items`) are deleted.
2. **Schemas by stage kind**, generated from Rust types with schemars (as Design Session does):
   - **Generator** (the source stage): `{ items: [{ key, description, artifact? }] }`.
   - **Producer / implementer transformer:** `{ artifact?, description? }`. There is no `key`: the item keeps its parent's key.
   - **Reviewer transformer:** `{ verdict: "approve" | "revise" | "reject", reason, artifact? }`. `reason` is required.

   Every field is a plain string or an enum. The CLI nudges only once, so the schemas are kept easy to satisfy.
3. **A missing `structured_output` is an operational failure.** There is no repair turn of our own; the CLI's nudge is the repair turn.
   - A transformer goes through `operational_failure`: it uses up an attempt and re-queues, reaching needs-human at 3 attempts.
   - The generator gets the activator's existing log-and-back-off.
   - A `structured_output` that fails to deserialise into the Rust type is treated the same way.
4. **Reviewer verdicts are acted on for every edge kind.** On a plain edge:
   - approve → commit to `on_approve`, as today;
   - revise → a new child at the `on_revise` target (the team that produced the item when `on_revise` is absent), with `attempts + 1`; at the attempts cap it escalates instead;
   - reject → `NeedsHuman` at the `on_reject` target (`needs-human` when absent).

   Gates and joins keep their current logic. They now get a verdict that is always present, so the engine's default-to-Approve goes away.
5. **The reason reaches the producer.**
   - On every reviewer verdict, the reason is stored as a task comment of kind `review`. Its anchor is the reviewed artifact.
   - The revision bundle reads comments across the **item's lineage** (same `run_id` and `item_key`), not just the child task's id. This also fixes the human-comment gap.
   - Review comments appear under the existing `--- REVISION REQUEST (attempt N) ---` header, labelled with the reviewing team.
6. **`output_contract` loses the marker grammar** but keeps its guidance: the artifact folder, already-found keys for the generator, and the description length. It gains one line: "Report your result by calling the StructuredOutput tool."
7. **Scope never blocks `StructuredOutput`.** The worker settings that `scope.rs` writes always allow the `StructuredOutput` tool, and the Scope spec (#10) inherits that rule.
8. **The `anthropic-api` runner** returns an "unsupported" runner error for worker invocations. It is hidden from the UI and not maintained by this map. Its marker parsing is deleted with the rest.
9. **Cost.** The schema adds one turn per invocation (about $0.21–0.32 at list price on a trivial run, mostly cache creation). This is accepted: the turn is reported in `result.usage` and so is already counted in Cost and Utilization.

## Architecture

### Shared kernel (`agent_bus_core`)

`Verdict` is unchanged. The output types live here so that Runners and Runtime share them:

```rust
#[derive(Deserialize, JsonSchema)]
pub struct GeneratorOutput { pub items: Vec<GeneratedItem> }

#[derive(Deserialize, JsonSchema)]
pub struct GeneratedItem {
    pub key: String,
    pub description: String,
    pub artifact: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProducerOutput {
    pub artifact: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ReviewerOutput {
    pub verdict: Verdict,
    pub reason: String,
    pub artifact: Option<String>,
}

pub enum OutputKind { Generator, Producer, Reviewer }

pub enum WorkerResult {
    Generator(GeneratorOutput),
    Producer(ProducerOutput),
    Reviewer(ReviewerOutput),
}
```

`OutputKind::schema() -> serde_json::Value` produces each schema once, through schemars.

### Runners (ACL) — the only code that knows `--json-schema`

- `command.rs` adds `--json-schema=<compact JSON>` as a single argv element, using the `=` form.
- `RunnerRequest` gains `output_kind: OutputKind`.
- `StreamAccumulator.finish` reads `result.structured_output`:
  - When it is present, it is deserialised into the `WorkerResult` for that kind.
  - When it is absent, or doesn't deserialise, the result is the new `RunnerError::NoStructuredOutput { detail }`.
  - `final_text` keeps the assistant prose for the live log and the audit.
- `RunnerOutput.verdict` and `.artifact_path` are replaced by `result: WorkerResult`.
- `parse_verdict`, `parse_artifact`, `parse_items` and `OutputItem` are deleted.
- `output_contract` is rewritten per decision 6.
- `scope.rs` always includes `StructuredOutput` in the allow list.
- The `ErrorClass` mapping adds `no-structured-output`, so the audit records it as `error:no-structured-output`.

### Runtime

- `transform_once` and `generate_once` take their items from `WorkerResult` instead of `parse_items(final_text)`.
  - A transformer that gets `NoStructuredOutput` goes to `operational_failure`, like any runner error.
- **Routing a reviewer verdict on a plain edge** (decision 4) reuses the send-back and escalate steps from `apply_gate_verdict`. They are pulled into shared helpers so the plain-edge and gate paths behave the same.
- **Review comments.** On every reviewer result, a `kind = 'review'` comment is written with:
  - `task_id` = the reviewed task;
  - `artifact_path` = the reviewed artifact;
  - `note` = `"<team>: <verdict> — <reason>"`.
- **Revision bundle.** `RevisionReader` looks up comments by `(run_id, item_key)` across tasks, oldest first.
- The audit's verdict column takes the real verdict. Defaulting to Approve is removed.

### Composition root (app)

- `SqliteRevisionReader` queries by `(run_id, item_key)`, joining `comments` to `tasks`. No migration is needed: `comments.kind` and `tasks.run_id/item_key` already exist.

### Frontend

- Review comments show in the existing comments list with a "review" label and the reviewing team. There is no new view.

## Data flow

1. Engine builds the request: team prompt plus output contract, `output_kind` from the stage (source → Generator; reviewer role → Reviewer; otherwise Producer).
2. Runner argv gets `--json-schema=<schema>`. The CLI validates and nudges once if needed.
3. The `result` event gives `structured_output`, deserialised into a `WorkerResult`, or `NoStructuredOutput`.
4. Engine acts on the result:
   - Generator: deduplicate and cap at K slots, then enqueue the items.
   - Producer: commit downstream.
   - Reviewer: store the review comment, then route by verdict per edge kind.
5. On revise, the producer's next attempt gets a revision bundle that includes the reviewer's reason.

## Error handling

- **No `structured_output`, or it fails to deserialise:** treated as an operational failure (decision 3). The audit records `error:no-structured-output`. The prose stays in the live log for diagnosis.
- **Generator returns zero new keys:** dry, as today. An empty `items` array is a valid result, not a failure.
- **Reviewer revises at the attempts cap:** escalates to needs-human, as gates already do.
- **`StructuredOutput` blocked by a hook:** prevented by decision 7. If it happens anyway, it surfaces as `NoStructuredOutput`.

## Testing

- **Schemas:** a snapshot test for each `OutputKind` schema. Each schema is a valid JSON Schema with only string, enum, array and object types.
- **Accumulator:**
  - fixtures captured from real runs give the right `WorkerResult` for each kind;
  - a run ending `end_turn` with no `structured_output` gives `NoStructuredOutput`;
  - an object of the wrong shape gives `NoStructuredOutput`.
- **Argv:** `--json-schema=` is a single element, and the positional prompt survives.
- **Engine:**
  - a reviewer on a plain edge: approve forwards, revise creates a child at the producer with attempts + 1, revise at the cap escalates, reject escalates to `on_reject` / `needs-human`;
  - a transformer with `NoStructuredOutput` re-queues and reaches needs-human at 3;
  - gate and join behaviour is unchanged.
- **Revision bundle:** a reviewer's reason on a parent task appears in the child's bundle. Human comments on the parent appear too.
- **Scope:** generated settings allow `StructuredOutput`.
- **Live check (manual):** a two-team producer → reviewer pipeline on the installed CLI produces a review comment and routes it.

## Out of scope

- A repair turn of our own via `--resume`. If failures prove common, it can be revisited after the session-resume spec (#13).
- Prompt changes to the bundled DDD template beyond what `output_contract` already appends. This is still fog on the map.
- The `anthropic-api` runner's structured path (decision 8).
- New UI for verdicts beyond labelling review comments. The Run Inspector spec (#11) covers result metadata.
