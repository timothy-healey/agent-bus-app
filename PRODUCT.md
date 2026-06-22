# PRODUCT.md — Agent Bus App

## Register

**product** — design serves the work. This is a tool you use during the day to run multi-team Claude Code agent orchestrations. Not a landing page; not a marketing surface. The UI's job is to fade into the work, not to perform.

## Product purpose

A local app for orchestrating multi-team Claude Code agent pipelines. The user defines teams (each team = one prompt + scope + skill), connects them into a graph (forward edges, revise loops, gates, escalation routes), and the app runs workers, surfaces gated decisions, lets the user review documents and interact with reviewers on revisions, and tracks cost per task.

The bundled default is a DDD spec → plan → implement pipeline (research / spec / plan / implement / review at each stage, two human gates). Users can edit it or design their own.

## Users

**Primary:** a single technical operator running it locally. Someone who is comfortable with the terminal, has used Claude Code, and wants visibility + control over multi-agent work without managing 8 tmux panes.

**Secondary:** anyone who clones the repo to use the pattern for their own project. The app must be self-explanatory enough that "git clone, follow README, see your first pipeline run" is achievable without external onboarding.

There is no "team" or "organisation" use case in v1. One operator, one local install, one project at a time.

## Tone & personality

- **Calm, focused, deliberate.** The user is making non-trivial decisions (approving specs, reviewing diffs). The interface must not compete with the work.
- **Technical without being macho.** The user is technical, but the UI shouldn't shout "DEV TOOL" with neon dark mode and ASCII flourishes. Confidence, not aggression.
- **Quietly opinionated.** Has a default pipeline; has a colour for "needs you"; doesn't ask the user to pick the obvious things.
- **No telemetry vibe.** Despite showing token counts and worker counts, this is not a metrics dashboard. The metric *serves* the work; the work isn't a metric.
- **Crafted, not generated.** The look should make a person say "someone designed this," not "Claude made this."

## Anti-references — what this is NOT

- ❌ **GitHub Dark / generic dev-tool dark.** Slate background, blue/purple/green/amber/red semantic chips, monospace everywhere. Too generic; reads as AI-generated default.
- ❌ **Vercel/shadcn zeitgeist.** Slate + lime + sharp corners + Inter. Saturated; the second-order reflex for "modern AI tool."
- ❌ **Neon terminal aesthetic.** Black + green + glowing borders. Performative; macho.
- ❌ **Linear clone.** Indigo gradients on dark, big rounded corners. Familiar but predictable.
- ❌ **Datadog / business ops dashboard.** Light grey on white with bright-coloured status chips and dense tables. Reads as serious enterprise; wrong tone.
- ❌ **The hero-metric / SaaS landing template.** Big card grids, identical tile arrays.

## Strategic principles

1. **The work is the focus; the chrome is quiet.** Title bars, navigation, headers should be small and unobtrusive. Document content (specs, plans, reviews) should feel like reading, not skimming.
2. **State is communicated through restraint, not chips.** Six different states (queued / running / gated / revise / needs-human / braked) shouldn't be six different bright colours. Use position, weight, and one strategic accent for "needs you."
3. **Density follows from intent.** The Kanban is glanceable. The artifact viewer is reading-focused. Don't impose the same density on both.
4. **The pipeline IS visible by default.** Users shouldn't have to click around to know the shape of what's running. The default view shows the graph in motion.
5. **Cost is honest but not anxious.** Per-card token count tells you what's expensive. Don't make it red-by-default or surround it with warnings — just show the number, let the user judge.

## Constraints

- **Tauri + React + TypeScript + Vite.** WebKit on macOS, WebView2 on Windows.
- **Distribution** is `git clone && bun install && cargo tauri dev`. Anyone with rust + bun can run it.
- **Self-hostable** is not v1.
- **Theme:** to be settled in DESIGN.md — must be derived from a concrete scene sentence per impeccable's law, not "dev tools look cool dark."

## Scene sentence (for impeccable's theme decision)

> *Tim, mid-morning at his desk, has injected three topics into the bus. Two are working; one hit Gate 1. He glances at the app between IDE windows to see whether the spec at the gate is ready to read; if it is, he opens it side-by-side with VS Code, reads, approves or marks revisions, then goes back to the IDE while the next stage runs.*

This is a daytime, secondary-window, briefly-glanced tool that occasionally becomes a primary reading surface (when reviewing a doc at a gate). Theme has to serve both — easy to glance at, and pleasant to read in for 5-10 minutes.

## Voices and copy notes

- Error messages: terse, direct, no apologies. *"Worker died — check `~/agent-bus/logs/research.log`"* not *"Oh no! Something went wrong. We're sorry."*
- Empty states: not playful, not corporate. Just state what's missing and what to do. *"No topics yet. Drop one in `topics/injected/` or use the inject button."*
- Status pills: noun, not verb. *"running"* / *"needs you"* / *"revise"* — not *"is running"* / *"requires action"*.
- No em dashes in copy. No exclamation marks.
