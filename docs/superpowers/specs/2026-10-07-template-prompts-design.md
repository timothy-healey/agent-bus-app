# Spec — DDD template prompts for structured output and scopes

*Design doc. Wayfinder map: "Map: runner capabilities from the current Claude CLI" (#3). Ticket: "Spec: DDD template prompts for structured output and scopes" (#19). Builds on "Spec: structured verdicts" (#8) and "Spec: real scope enforcement" (#10). The bundled `ddd-spec-plan-impl` template's seven prompts are rewritten for structured output and real scopes. Each worker's user message also states its input explicitly: the item, the artifact it carries, and the worktree. A reviewer's approval forwards the reviewed artifact, not the reviewer's own file. Terms are as defined in `DOMAIN.md` (Pipeline Authoring, Runtime, Runners).*

## Why this exists

- **The prompts contradict the scopes.** The implementer prompt says "NEVER push, fetch, or otherwise touch any remote". The scope spec grants implementers and code-reviewers Remote git, and in auto mode the classifier only allows a push the task asked for.
- **Workers usually never see their input.** A transformer child's `topic` is its one-line description (`topic_for_item`). That topic is non-empty, so `fallback_user_message` — the only place the input artifact path appears — never runs. Spec-writers, both kinds of reviewer, implementers and code-reviewers usually get a single line of description and no path.
- **The wrong artifact moves forward.** A reviewer's own artifact, or its default path, becomes the downstream item's input, so plan-writers after the spec gate receive the spec-reviewers' file, not the spec.
- **Code-reviewers aren't told what to review.** They run inside the implementer's worktree, but nothing tells them the branch or what to diff against.
- **Reviewers don't know what a reason is for.** Their prompts say nothing about what a `reason` should contain, though under structured verdicts it becomes the producer's revision brief.

## Decisions

1. **Every worker's user message starts with an input block**, built by the engine:

   ```
   ## Input
   Item: <key> — <description>
   Input artifact: <absolute path>            (omitted for the generator)
   Run artifacts: <absolute artifacts root>
   Worktree: <absolute path> — branch <branch>  (only when the task has a worktree)
   ```

   The revision bundle (structured verdicts, decision 5) follows it unchanged. `fallback_user_message` is removed: the input block is never empty. The generator's block has only `Run artifacts`.
2. **The item's artifact is the latest producer artifact.**
   - A reviewer's verdict never replaces it. On approve, the downstream child's input artifact is the reviewed artifact.
   - A reviewer's optional `artifact` (a longer review note) is linked from its review comment instead.
   - The computed default path for a missing artifact remains only for producers.
3. **Remote work happens only when the task asks.**
   - Implementers commit in their worktree. They push their branch and open a **draft** PR only when the item's topic or the revision bundle asks for one.
   - Code-reviewers review the local worktree diff. They check out a PR (`gh pr checkout`) only when given a PR number.
   - Neither merges. The prompts say this in plain words, so auto mode sees the intent.
4. **Prompts keep naming skills.** Declaring a plugin only loads it, so the prompt is still what tells the worker to use a skill. Each skill named must belong to a plugin the team declares (scope spec, decision 9).
5. **The reviewer's `reason` is a brief for whoever acts next.**
   - revise: a numbered list of concrete changes;
   - reject: why the item is unworkable, in one paragraph;
   - approve: one sentence on what makes it sound.
6. **The prompts don't repeat the output contract.** The schema and `output_contract` carry the mechanics; prompts describe the job, the input and what good output looks like. No prompt mentions markers, keys or schemas.
7. **The template description** drops "(no push)" and says "implementers work in a local git worktree and open a draft PR only when asked".

## The prompts

Each prompt below replaces `prompt_body` in `seed_template.rs` verbatim.

**research** (generator)

> You find concrete, well-scoped units of work in the target repository for this delivery. Read the repository and any existing run artifacts. For each candidate, write a short analysis artifact: where it is, why it is a candidate, and the proposed change. Aim for whole-scope coverage and give each candidate a stable, descriptive key. You are read-only on the repository — do not modify code.

**spec-writers** (producer)

> Your input artifact is a candidate analysis. Write a change specification for it. Invoke the superpowers brainstorming skill and take its recommended defaults without pausing for questions. The spec states the target, the proposed change, the methods and call sites affected, and explicit acceptance criteria. If your input includes a revision request, address every point in it. Write the spec as a Markdown artifact; do not implement.

**spec-reviewers** (reviewer)

> Your input artifact is a change specification. Review it for viability and soundness: is the scope right, is it testable, is the approach correct, do the acceptance criteria match the change? Approve a sound spec. Request revision when it can be fixed, with a numbered list of the concrete changes needed as your reason. Reject only when it is unworkable, and say why.

**plan-writers** (producer)

> Your input artifact is an approved change specification. Turn it into an implementation plan by invoking the superpowers writing-plans skill: TDD, bite-sized tasks, exact file paths, frequent commits. If your input includes a revision request, address every point in it. Write the plan as a Markdown artifact; do not implement.

**plan-reviewers** (reviewer)

> Your input artifact is an implementation plan. Vet it with /ddd-council vet for domain soundness — boundaries, aggregates, the right seams — and review it as an implementation plan: sequencing, testability, completeness against its spec, which is among the run artifacts. Approve a sound plan. Request revision with a numbered list of concrete changes as your reason. Reject only when it is unworkable, and say why.

**implementers** (implementer)

> Your input artifact is an approved implementation plan. Implement it in your git worktree using the superpowers subagent-driven-development skill (TDD, with a two-stage review per task). Commit your work on the worktree's branch. Push the branch and open a draft pull request only if your item or revision request asks for one; otherwise keep everything local. Never merge, never force-push, never push to main or master. If your input includes a revision request, address every point in it. Write a short artifact summarising what you changed and how you tested it.

**code-reviewers** (reviewer)

> Your input artifact is the implementer's summary; the changes are on the worktree's branch. Review the diff between that branch and its merge-base with the repository's default branch against the plan and spec in the run artifacts: conformance, code quality, test coverage. If you are given a pull-request number, check it out with `gh pr checkout` and review that instead. Never merge. Approve finished work. Request revision with a numbered list of concrete changes as your reason. Reject only when it is unworkable, and say why.

## Architecture

### Runtime

- `compose_invocation_message` takes an `InvocationInput { key, description, input_artifact, artifacts_root, worktree: Option<(path, branch)> }` and renders decision 1's block before the revision bundle.
  - The branch is read from the worktree when the message is built (`git -C <path> branch --show-current`), with no new stored field.
  - If git fails, the line is left out.
- `fallback_user_message` is deleted.
- **Input artifact for children:**
  - Children of a producer carry the producer's artifact (as today).
  - Children of a reviewer carry the reviewer's own input artifact. This covers the plain-edge, gate and join paths, through the shared send-back and commit helpers from the structured-verdicts spec.

### Pipeline Authoring

- `seed_template.rs`: the seven prompts and the description above.
- The seed's tests check the following:
  - every skill a prompt names (`superpowers …`, `/ddd-council`) belongs to a plugin the team declares;
  - no prompt mentions `KEY:`, `VERDICT:`, `ARTIFACT:` or "StructuredOutput".

### Frontend

- No changes. The `playwright` mirror of the template updates its description if it asserts on it.

## Testing

- **Input block:**
  - generator, plain transformer, gate child and worktree child each render the expected block;
  - the revision bundle follows it;
  - a failed branch lookup leaves out the branch line.
- **Artifact passing:** a reviewer approve on a plain edge, through a gate, and into a join each give the downstream child the producer's artifact. The reviewer's `artifact` appears only in its review comment.
- **Seed:** the prompt and plugin consistency check; the no-markers check; the description text.
- **Live check (manual):** a run of the template on a scratch repo. Spec-reviewers' reasons arrive in the spec-writers' revision bundle, plan-writers read the spec (not the review), and implementers commit without pushing unless asked.

## Out of scope

- Prompt generation for user-made pipelines (the kickoff "Generate" flow). This spec changes only the bundled template.
- An app-level "open PRs" pipeline setting. Intent comes from the item or the revision request.
- Routes for producers' `on_revise` / `on_reject`. Under structured verdicts only reviewers' verdicts route.
