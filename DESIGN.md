# DESIGN.md — Agent Bus App

Design tokens, components, and visual rules for the app. Derived from the impeccable-led design session (see `assets/designs/` for screenshots and `assets/html/` for the source mockups). The committed direction is **T1 — Full Mono, Warm Dark** with a **Warm Light** mode for daylight use.

## Theme

- **Default theme: warm dark.** Brown-tinted near-black; not blue-grey, not pure black. The IDE-adjacent companion choice.
- **Alternate theme: warm light.** Off-white tinted yellow-warm; for daytime reading, beside a dark IDE.
- **Theme is a user toggle.** Default on first launch follows OS dark-mode preference, then sticks.

The scene that anchored the theme decision:

> *Tim, mid-morning at his desk, has injected three topics into the bus. Two are working; one hit Gate 1. He glances at the app between IDE windows to see whether the spec at the gate is ready to read; if it is, he opens it side-by-side with VS Code, reads, approves or marks revisions, then goes back to the IDE while the next stage runs.*

Daylight and IDE-adjacency forced light-as-an-option; mono-as-the-voice came from impeccable's product mode permitting density and earned familiarity without category cliché.

---

## Color tokens (OKLCH)

Per impeccable's law: never `#000` or `#fff`. Every neutral is tinted toward the brand hue. Restrained palette — tinted warm neutrals + **one** ochre accent.

### Dark (default)

```css
--bg:         oklch(11%  0.005 60);  /* page background — warm near-black */
--bg-2:       oklch(9%   0.004 60);  /* nav, terminal, table headers */
--surface:    oklch(15%  0.005 60);  /* cards */
--surface-2:  oklch(18%  0.006 60);  /* hovered cards, mini-board cards */
--surface-3:  oklch(22%  0.008 60);  /* popovers, elevated UI */
--border:     oklch(24%  0.008 60);  /* default dividers */
--border-2:   oklch(32%  0.010 60);  /* card hover, node borders */
--text:       oklch(92%  0.008 70);  /* primary text */
--text-2:     oklch(72%  0.008 70);  /* secondary text, button labels */
--text-3:     oklch(55%  0.008 70);  /* metadata, lane labels */
--text-4:     oklch(42%  0.008 70);  /* disabled, placeholder */
```

### Light (alternate)

```css
--bg:         oklch(98%  0.005 70);
--bg-2:       oklch(96%  0.006 70);
--surface:    oklch(99.5% 0.003 70);
--surface-2:  oklch(96%  0.006 70);
--surface-3:  oklch(93%  0.008 70);
--border:     oklch(89%  0.008 70);
--border-2:   oklch(82%  0.008 70);
--text:       oklch(22%  0.012 70);
--text-2:     oklch(45%  0.010 70);
--text-3:     oklch(60%  0.008 70);
--text-4:     oklch(72%  0.006 70);
```

### Semantic tokens (both themes)

The one accent — **ochre** — is the only saturated colour in everyday use. Reserved for "needs you" gates, primary actions, focused state. Should occupy ≤10% of the surface area.

```css
/* DARK */
--accent:     oklch(74%  0.13  55);   /* ochre — lifted for contrast on dark */
--accent-2:   oklch(20%  0.040 55);   /* surface tint */
--accent-3:   oklch(28%  0.060 55);   /* hover state on accent surface */
--accent-bd:  oklch(48%  0.090 55);   /* border on needs-you cards */

/* LIGHT */
--accent:     oklch(58%  0.135 55);   /* ochre — deeper for contrast on light */
--accent-2:   oklch(96%  0.030 70);
--accent-3:   oklch(92%  0.050 60);
--accent-bd:  oklch(80%  0.060 60);
```

State indicators — used as 6px dots, not pills. Sparingly.

```css
/* DARK */
--running:    oklch(72%  0.08  145);  /* muted sage */
--running-2:  oklch(22%  0.030 145);  /* halo around running dot */
--revise:     oklch(70%  0.10  305);  /* muted purple — bounce-back */
--revise-2:   oklch(20%  0.035 305);
--danger:     oklch(70%  0.12  25);   /* used for reject action only */
--danger-2:   oklch(22%  0.04  25);

/* LIGHT */
--running:    oklch(48%  0.060 145);
--running-2:  oklch(92%  0.020 145);
--revise:     oklch(48%  0.090 305);
--revise-2:   oklch(96%  0.020 305);
--danger:     oklch(48%  0.13  25);
--danger-2:   oklch(96%  0.02  25);
```

---

## Typography

Per impeccable's product mode: one type family is often right; system fonts are legitimate. T1 chose **mono everywhere** as a deliberate voice — distinct from default-Inter SaaS without being terminal-cliché.

### Font stack

- **Primary (UI chrome, labels, buttons, IDs, numbers):** `"Berkeley Mono", "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, monospace`
- **Reading body (artifact rendered content — specs, plans, reviews):** `ui-sans-serif, -apple-system, BlinkMacSystemFont, system-ui, sans-serif`
- Reading-body is used **only** for rendered markdown inside the artifact viewer. Everywhere else in the app, mono.

The two-family combination is justified: long-form markdown content set in mono is fatiguing; the chrome itself benefits from the deliberate, evenly-spaced voice mono provides.

### Font features

```css
font-feature-settings: 'ss01', 'ss02';  /* JetBrains Mono stylistic alternates */
font-variant-numeric: tabular-nums;     /* applied to all numeric data */
-webkit-font-smoothing: antialiased;
```

### Type scale

Mono UI scale — 1.18 ratio:

| Role | Size | Weight | Line-height |
|---|---|---|---|
| `--ts-xs` | 10px / 0.625rem | 400 | 1.4 |
| `--ts-sm` | 11px / 0.6875rem | 400 | 1.45 |
| `--ts-base` | 12.5px / 0.78rem | 400 | 1.5 |
| `--ts-md` | 13px / 0.8125rem | 450/500 | 1.5 |
| `--ts-lg` | 14px / 0.875rem | 500 | 1.4 |
| `--ts-xl` | 16px / 1rem | 500 | 1.35 |

Reading body (sans-serif, inside artifact viewer):

| Role | Size | Weight | Line-height |
|---|---|---|---|
| body | 13.5px | 400 | 1.6 |
| h3 | 14px | 600 | 1.3 |
| h2 | 16px | 600 | 1.3 |
| h1 | 19px | 600 | 1.3 |

Body line length cap: **75ch**.

### Letter-spacing

- UI labels and small caps: `letter-spacing: 0.03em`–`0.05em` for column heads, status pills
- Default body: `letter-spacing: 0` (mono is already evenly-spaced)
- Reading body: `letter-spacing: -0.005em` for large headings

---

## Spacing

4px base unit. Scale: **4 · 6 · 8 · 10 · 12 · 14 · 16 · 18 · 20 · 24 · 28 · 32 · 48 · 64**.

```css
--sp-1: 4px;   --sp-2: 8px;   --sp-3: 12px;  --sp-4: 14px;
--sp-5: 16px;  --sp-6: 18px;  --sp-7: 20px;  --sp-8: 24px;
--sp-9: 28px;  --sp-10: 32px; --sp-12: 48px; --sp-14: 64px;
```

Page-level horizontal padding: 32px. Card internal padding: 10–14px. Lane gap: 12px.

---

## Radius

Restrained — small radius is part of the voice. Big rounded corners read as Linear-clone.

```css
--r-xs: 2px;   /* buttons, small pills */
--r-sm: 3px;   /* inputs, popover, small surfaces */
--r-md: 4px;   /* cards, drawer container */
--r-lg: 6px;   /* (used sparingly — rare) */
--r-pill: 999px;  /* pipeline-name badge only */
```

---

## Elevation / shadow

Dark mode shadows must be saturated enough to read against the warm dark surface.

```css
--shadow-card-dark: 0 1px 2px oklch(0% 0.01 70 / 0.20),
                    0 4px 12px oklch(0% 0.01 70 / 0.30);
--shadow-card-light: 0 1px 2px oklch(20% 0.01 70 / 0.04),
                     0 4px 12px oklch(20% 0.01 70 / 0.03);
--shadow-popover: 0 4px 20px rgba(0,0,0,0.5);
--shadow-needs-you-dark: 0 1px 2px oklch(74% 0.135 55 / 0.15),
                        0 6px 16px oklch(74% 0.135 55 / 0.18);
--shadow-needs-you-light: 0 1px 2px oklch(58% 0.135 55 / 0.10),
                         0 6px 16px oklch(58% 0.135 55 / 0.08);
```

---

## Motion

Per impeccable: 150–250ms on most transitions; ease-out exponential. No bounce, no elastic. Motion conveys state, never decoration.

```css
--ease-out: cubic-bezier(0.16, 1, 0.3, 1);   /* ease-out-expo */
--dur-fast: 100ms;       /* hover transitions */
--dur-base: 150ms;       /* default */
--dur-slow: 250ms;       /* drawer slide, board re-layout */
```

Allowed animations:

- **Running dot pulse:** `pulse 1.8s ease-out infinite` — opacity 1 → 0.55 → 1
- **Needs-you banner pulse:** `pulse 1.5s ease-out infinite` (dot only)
- **Card hover lift:** `transform: translateY(-1px); border-color: var(--border-2);` over 150ms
- **Drawer slide-in:** translateX from +100% to 0 over 250ms with `--ease-out`
- **Card bounce-back to plan-writers lane:** position transition over 350ms with `--ease-out`

Forbidden:

- Animating layout properties (width, height, padding)
- Bouncy/elastic curves
- Page-load orchestration

---

## Components

### Card

The unit of the Kanban. Same template; visual state varies.

**Structure:**
```
┌─────────────────────────────────┐
│ T-042              ● running    │  ← top row: id + state pill
│                                 │
│ Patient Records bulk-write      │  ← title (mono, 13px, weight 450)
│                                 │
│ 4m 23s · 47k · w 1/2    a1     │  ← meta row: age · cost · worker · attempts
└─────────────────────────────────┘
```

States — colour comes from one dot, the rest is layout:

| State | Border | Background | Dot colour | Note |
|---|---|---|---|---|
| queued | `var(--border)` | `var(--surface)` | `var(--text-3)` | — |
| running | `var(--border)` | `var(--surface)` | `var(--running)` + halo | dot pulses |
| **needs you** | `var(--accent-bd)` | `var(--surface)` | `var(--accent)` | + needs-you shadow |
| revise (queued) | `var(--revise)` `0.5 opacity` | `var(--revise-2)` | `var(--revise)` | shows ↩ marker on id |
| canon q | `var(--danger-2)` border | `var(--surface)` | `var(--danger)` | rare; only escalations |
| braked | `var(--border)` | `var(--surface)` `0.6 opacity` | `var(--text-4)` | dimmed |

Padding: 10px 12px. Margin between cards in a lane: 6px. Radius: `--r-md`.

### Button

Three variants. Mono throughout. Small radius.

| Variant | Background | Border | Text | When |
|---|---|---|---|---|
| `default` | `var(--surface-3)` | `var(--border-2)` | `var(--text)` | secondary actions |
| `primary` | `var(--accent)` | `var(--accent)` | `oklch(15% 0.04 55)` | the recommended action |
| `ghost` | transparent | `var(--border)` | `var(--text-2)` | cancel, dismiss |
| `danger` | transparent | `oklch(35% 0.05 25)` | `var(--danger)` | reject; rare |

Padding: 7px 14px (default) · 4px 10px (small `.btn-sm`). Hover: `border-color: var(--border-2)`.

### Drawer (slide-over)

Container for card detail. Slides in from the right at `--dur-slow`. Width: 60% of viewport (clamped 600px–960px).

**Structure:**
```
┌──────────────────────────────────────┐
│ T-040       plan-reviewers · v1 · 154k │  ← head
│ Scheduling bulk-write — plan v1       │
├──────────────────────────────────────┤
│ artifact² │ live log │ review │ lineage │  ← tabs (badge shows comment count)
├──────────────────────────────────────┤
│                          │ comments  │
│ (rendered artifact body) │   rail    │  ← body, scrollable
│                          │ ① ②       │
├──────────────────────────────────────┤
│ 2 comments · ready   reject revise APPROVE │  ← action bar
└──────────────────────────────────────┘
```

Action-bar buttons: ordered by destructiveness — `reject` (danger) on the left, then `revise`, then primary `approve` on the right. When comments exist, `revise` becomes primary; `approve` becomes default. Single rule: the primary button is the recommended action given current state.

### Comment system

Two visual surfaces, one language:

- **Inline anchor (in the artifact body):** ochre underline `1.5px var(--accent)`, ochre surface tint `var(--accent-2)`, with a `marker` superscript circle (14px, ochre fill, dark text, monospace numeral).
- **Rail entry (in the right rail):** circle marker (16px) + meta line + quoted snippet (sans-serif, italic, left-rule in `--border`) + your note (sans-serif body).

Clicking either surface highlights both (the inline anchor moves to `var(--accent-3)` background, the rail entry adds `var(--accent-2)` background).

### Popover (selection-comment composer)

Triangle-pointer top-left, anchored to selection. Background `var(--surface-3)`, border `var(--border-2)`, shadow `--shadow-popover`. Same quoted-snippet treatment as rail entries. Same button styles. The popover and rail entry are interchangeable visually — they're the same UI in two states (composing vs saved).

### God terminal (bottom dock)

Always-present at the bottom of the main view. Collapsible to a thin bar.

**Structure (expanded):**
```
┌──────────────────────────────────────┐
│ ● claude    context: full pipeline   │  ← head: 28px, status + ctx
├──────────────────────────────────────┤
│ you · 09:42                          │
│ inject 03-scheduling once T-042…     │  ← message stream
│                                       │
│ claude                                │
│ T-042 is mid-research…                │
│ → watch_outbox  → inject_topic        │  ← tool calls inline
├──────────────────────────────────────┤
│ › ask, inject, approve, brake…  ↑ ⌘K │  ← input row
└──────────────────────────────────────┘
```

Max height: 280px. Input prompt: `›` in `var(--accent)`. Hints (`↑ history`, `⌘K commands`) right-aligned in `var(--text-3)`.

**Tool-call chips** render inline within Claude's messages. Shape: monospace, `var(--surface)` background, `var(--border)` border, 5px radius, 3px 10px padding. Arrow `→` in `var(--accent)`. Status: green `✓` for ok, `⏳` for pending.

### Pipeline editor

Dot-grid canvas (`radial-gradient(circle at 1px 1px, var(--border) 1px, transparent 0)` at 16px intervals). Node colours by role:

| Node type | Border |
|---|---|
| writer | `oklch(40% 0.07 240)` (cool blue) |
| reviewer | `oklch(45% 0.09 300)` (cool purple) |
| gate | `var(--accent-bd)` (ochre) |
| impl | `oklch(45% 0.10 50)` (warm orange) |

Edges:

- **Forward** (approve): straight line, `oklch(40% 0.008 60)`, arrow head, 1.5px solid
- **Revise** (back-edge): curved Bezier from reviewer → upstream writer, `var(--revise)`, **dashed** `4,3`, arrow head
- **Escalate**: straight line to a `needs-human` sink, `var(--danger)`, dashed

Sidebar palette on the right: node-type chips that can be dragged onto the canvas. Selected-node config below: prompt path, skill, routing rules. All editable.

### Usage meter (topbar widget)

Lives top-right of the topbar between the pipeline name and the brake state. Shows rolling 5h window consumption.

**Structure (single line):**
```
[██░░░░░░] 47% · 5h window · 1.2M tok  ·  burn 18k/min
```

**Components:**
- **Bar** — 100px × 6px, `--surface-3` track, gradient fill `--running` → `--warn` → `--danger` at positions 0% / 60% / 90%. Width = current % of window.
- **% label** — `--text` colour at default state; becomes `--warn` at ≥60%, `--danger` at ≥85%.
- **Window meta** — "5h window · 1.2M tok" in `--text-3`, tabular numerals.
- **Burn rate** — "burn 18k/min" in `--text-3`, with number in `--text-2`. Becomes `--danger` at >25k/min.

**States:**
- **safe** (<60%): bar in `--running` only, no colour shift on labels
- **warn** (60–85%): gradient up to amber, % label amber, tooltip estimates brake-at
- **hot** (≥85%): bar mostly red, % label red, meter border subtly red-tinted
- **braked**: meter dims (opacity 0.6), border red, replaces burn-rate with `↻ <time until window resets>`

**Tooltip on hover** — 280px width, surface-3, shows:
- Tokens this window (used / configured budget)
- Window start time + elapsed
- Burn rate (1-min and 10-min avg)
- Estimated brake-at time (in warn/hot states)
- Cost this session (api-runners only)
- Per-team breakdown: team name · model · effort · tokens · % of total

**Data flow — two sources, three views:**

- **Window total (meter bar + main number)** comes from polling Claude Code transcripts at `~/.claude/projects/**/*.jsonl` (FSEvents/inotify-watched), **plus** our own Anthropic-SDK responses for `api-runner` teams. This captures ALL usage on the machine — including Claude Code chats outside our app, which still consume the user's subscription window.
- **Per-team breakdown (tooltip)** comes from our own stream-json tail (we know team + task + model for each invocation we make).
- **Per-card cost (Kanban card meta)** comes from our own stream-json tail, attributed to the task.

**Dedup:** each transcript message has a stable `message_id`; the watcher only emits rows for unseen ids. Crash-safe across app restarts.

**Schema:**
```sql
-- our own attributed log
CREATE TABLE worker_usage_log (
  ts INTEGER, team TEXT, task_id TEXT, model TEXT,
  input_tokens INTEGER, output_tokens INTEGER,
  cache_creation INTEGER, cache_read INTEGER
);
-- mirror of Claude Code's usage; broader scope
CREATE TABLE cc_usage_log (
  ts INTEGER, message_id TEXT UNIQUE, model TEXT,
  input_tokens INTEGER, output_tokens INTEGER,
  cache_creation INTEGER, cache_read INTEGER
);
```

Why not just call `claude --print "/usage"`? It costs tokens to invoke and returns prose, not structured data. Polling transcripts is cheaper, more accurate, and event-driven.

**Configurable budget:** default 2.6M tokens / 5h (rough Pro plan estimate). User adjusts in Settings → Usage to match their actual subscription (Max plan ≈ 8M, API users set their own monthly target).

### Table (list view)

```css
font-size: 12px;
border-collapse: collapse;
```

- Header: `var(--bg-2)` background, `--text-3` colour, `text-transform: lowercase`, `letter-spacing: 0.02em`, font-size 11px
- Row hover: `background: var(--surface)`, cursor: pointer
- `needs-you` row: `background: oklch(18% 0.025 55)` (subtle ochre tint), plus the state dot + label in the state column. **No side-stripe** — reconciled with §Anti-patterns (2026-06-24): the tint + dot + label carry the signal, and the global ban on accent side-stripes >1px wins. (Earlier drafts of this spec called for a `border-left: 2px solid var(--accent)` first cell; that contradicted §Anti-patterns and has been removed.)
- Tabular numerals applied via `font-variant-numeric` on numeric columns

---

## States — full inventory for components

Every interactive component must define these, per impeccable's product law:

| State | Visual cue |
|---|---|
| default | base styles |
| hover | `border-color: var(--border-2)`, optional `translateY(-1px)` |
| focus | `outline: 2px solid var(--accent)`, `outline-offset: 1px`, no rounded outline |
| active (pressed) | `transform: translateY(0)`, slightly darker background |
| disabled | `opacity: 0.5`, `cursor: not-allowed`, no hover effect |
| loading | skeleton text in `var(--surface-2)` with a slow pulse — **never spinners in content** |
| selected | `background: var(--accent-2)`, `border-color: var(--accent-bd)` |
| error | `border-color: var(--danger)`, error message below in `var(--danger)` |

Skeletons for loading: never spinners in content areas. A spinner is allowed only on a button while its action is executing.

---

## Anti-patterns (from impeccable; codified here)

Hard rules — match-and-refuse during implementation review.

- ❌ **No side-stripe borders** as accent. Status communicated via dot + label + (rarely) full border.
- ❌ **No gradient text.** Solid colours only.
- ❌ **No glassmorphism** as default. Frosted glass is forbidden in v1.
- ❌ **No identical card grids** as the page layout. Cards are the unit *inside* lanes; the page layout is not a card grid.
- ❌ **No display fonts in UI labels.** Mono UI; sans body inside the artifact viewer is the only secondary family.
- ❌ **No em dashes** in UI copy. Period.
- ❌ **No exclamation marks** in operator-facing UI text.
- ❌ **No more than one accent.** Ochre is the only saturated colour permitted as a UI accent. Running-sage and revise-purple are state dots, **not** accents — they don't extend to borders, fills, or buttons.

---

## Implementation notes for the spec

1. **Tauri 2.x + React 18 + Vite + TypeScript.** Stylesheet language: vanilla CSS with custom properties for tokens. No CSS-in-JS framework — Tauri performance benefits from light-weight CSS pipeline.
2. **Token file lives at** `src/styles/tokens.css` and is imported once at app root. Theme switching toggles `data-theme="dark"` / `data-theme="light"` on `<html>`.
3. **Font loading:** ship Berkeley Mono and Inter Variable as self-hosted woff2 files in `public/fonts/`. Berkeley Mono is paid; if budget is a constraint, JetBrains Mono is the open-source substitute and is the secondary fallback in the stack.
4. **Component primitives:** build `<Card>`, `<Drawer>`, `<Button>`, `<CommentAnchor>`, `<RailEntry>`, `<NodeCanvas>`, `<Terminal>` first; the Kanban, list view, and pipeline editor compose these.
5. **Accessibility:** maintain `contrast >= 4.5:1` for body text (verified for both themes). Focus ring `var(--accent)` is visible on both. Reduced-motion media query disables all `transform`/pulse animations.

---

## Asset references

- **Screenshots:** `assets/designs/01-warm-light-kanban.png` through `08-topbar-usage-meter.png`
- **HTML source mockups:** `assets/html/` (each iteration preserved)
- **Original spec:** `~/splose/agent-bus-design.md` (file-queue era — predecessor system, useful for understanding the data model)
- **Brainstorm directory:** `~/assistant/.superpowers/brainstorm/...` (transient; copies in `assets/html/` are durable)
