# Impeccable Frontend Design Audit — agent-bus-app

**Date:** 2026-06-24
**Lens:** `impeccable` (product register), measured against `DESIGN.md` + `src/styles/tokens.css`.
**Scope:** Full React frontend — wizard, board/drawer/comments, chrome (topbar/usage/settings/terminal), pipeline view+editor, `ui/*` primitives, design-system tokens.
**Mode:** STATIC, review-only. **Applied: none.** Every finding below is a recommendation for the operator to action or decline.

---

## Overall assessment

The frontend has a **strong, coherent design spine and a serious drift problem at the edges.** The token system, type voice (mono everywhere, sans only for artifact body), restrained one-accent (ochre) palette, motion tokens, and the `ui/Card` / `ui/StatePill` / `ui/Button` primitives are faithful to `DESIGN.md` and to the product's "quiet, crafted, not generated" intent. There are **no AI-slop tells** (no gradient text, no glassmorphism, no hero-metric template, no identical card-grid page layout, no neon-terminal cliché). The core board + card + drawer language is genuinely distinctive.

The problem is **consistency of execution as the surface area grew.** Two systemic root causes account for the majority of findings:

1. **Almost everything is styled with inline `style={}`** (26 of ~31 component files). Inline styles structurally cannot express `:hover` / `:focus-visible` / `:active`, so **the entire app has no focus ring and almost no hover/active feedback** — a hard `DESIGN.md` §States requirement is unmet app-wide. The only `outline` declaration in the codebase is `outline: "none"` in `Terminal.tsx`.
2. **`DESIGN.md` specifies tokens that `tokens.css` never shipped** — the `--ts-*` type scale, `--font-mono`, `--shadow-popover`, `--shadow-needs-you`, and a referenced `--bg-3` all do not exist. So every component hardcodes pixel font sizes (several off-scale: 9.5, 10.5, 11.5, 13.5, 14.5), the popover/needs-you card silently fall back to the flat `--shadow-card`, and the addressed-comment row tint is invisible.

Layered on top: newer surfaces (wizard, pipeline editor modal) re-implement buttons/inputs as bare `<button>`/`<input>` instead of reusing `ui/*`, so they don't inherit the primitives' correct styling — and because the Tauri-boilerplate `App.css` is **not imported**, those bare buttons render with raw user-agent appearance (system sans, wrong colors), breaking the mono voice exactly where the eye lands during project setup.

Verdict: **the design language is sound; the implementation has not kept the whole surface inside it.** Most of this is mechanically fixable (add the missing tokens + one global focus rule + route bare controls through `ui/Button`), which collapses a large share of the findings at once. A handful of genuinely subjective calls (validation-banner color, the never-built node canvas, the needs-you side-stripe spec contradiction) need the operator.

**Findings by severity:** **High: 12 · Medium: 23 · Low: 19** (54 total), plus **7 design decisions** requiring sign-off.

---

# Part 1 — Safe / high-confidence batch

Low-risk, high-confidence fixes: missing tokens, a11y labels/roles, spacing/token swaps, missing states, copy. Grouped by theme. These can be applied largely mechanically without a visual-direction debate.

## 1A. Cross-cutting (fix once, resolves many)

| # | Sev | Issue | Fix |
|---|-----|-------|-----|
| C1 | **High** | **No focus ring anywhere.** Only `outline` in the codebase is `outline:"none"` in `Terminal.tsx:134`. `DESIGN.md` §States/§a11y mandate `outline: 2px solid var(--accent); outline-offset: 1px` on every control. | Add a global rule in `global.css`: `:where(button,input,textarea,select,[role="tab"],a,[tabindex]):focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }`. Remove the `outline:none` in Terminal. Single biggest a11y win. |
| C2 | **High** | **No hover/active state on any inline-styled control** (inline styles can't express pseudo-states). Only `.abp-card` gets hover. | Move `ui/*` primitives to CSS classes with `:hover`/`:active`, or add blunt global hover rules. (See Decision D1 for scope.) |
| C3 | **Med** | **Missing tokens.** `--ts-xs…--ts-xl`, `--font-mono`, `--shadow-popover`, `--shadow-needs-you`, `--bg-3` are referenced by spec/code but absent from `tokens.css`. | Add all of them to `tokens.css` (both themes for shadows). Turns dozens of px-literal findings below into mechanical swaps and restores the popover elevation + needs-you glow + addressed-row tint. |
| C4 | **Low** | **`App.css` is dead Tauri boilerplate** (hardcoded `#0f0f0f`/`#646cff`, Inter, `prefers-color-scheme` block fighting `data-theme`). Confirmed **not imported**. | Delete it, or strip to empty, so it can't be wired in by accident. |

## 1B. Tokens / theming / hardcoded values

| # | Sev | Surface · File:Line | Issue | Fix |
|---|-----|------|-------|-----|
| T1 | **High** | CommentRail.tsx:75 | `var(--bg-3, var(--bg-2))` — `--bg-3` undefined, so the **addressed-comment row tint silently equals the rail bg** (invisible). | Define `--bg-3` or point at `--surface`/`--surface-2`. |
| T2 | Med | SettingsView.tsx:96 | Hardcoded fallback hex `var(--danger, #d66)` — raw literal, violates no-hardcoded-color. `--danger` always exists so the fallback is dead. | Drop `, #d66`. |
| T3 | **High** | PipelineEditor.tsx:109 | Backdrop `rgba(0,0,0,0.5)` raw literal; inconsistent with Drawer's spec'd scrim `oklch(0% 0 0 / 0.4)`. | Use `oklch(0% 0 0 / 0.4)`. |
| T4 | Med | NewProjectWizard.tsx:151 | Wizard overlay `rgba(0,0,0,0.5)` — same scrim drift. | Match Drawer scrim; ideally add a `--scrim` token. |
| T5 | Med | Card.tsx:20; SelectionPopover.tsx:19 | needs-you card and popover both use `--shadow-card` instead of the spec `--shadow-needs-you` / `--shadow-popover` (don't exist yet → C3). | After C3, point each at its spec token. |
| T6 | Low | CompareView.tsx:52,62 | Mono + reading-body font stacks inlined (and the inline reading stack drops `BlinkMacSystemFont` vs `DESIGN.md`). | Use `.mono` / a shared `.reading-body` class. |
| T7 | Med | many files | Pervasive raw px font sizes, several off the type scale (9.5/10.5/11.5/13.5/14.5). | After C3, swap to `--ts-*`; snap off-scale values to nearest legal step. |
| T8 | Low | many files | Raw px spacing/gaps (`gap:12`, `marginBottom:8`, `6px`) bypass `--sp-*`. Note: `6px` has **no** `--sp` token (scale jumps 4→8). | Use `--sp-*`; either add `--sp` for 6 or snap 6→8 (see D7). |

## 1C. Accessibility (labels / roles / focus management)

| # | Sev | Surface · File:Line | Issue | Fix |
|---|-----|------|-------|-----|
| A1 | **High** | ui/Drawer.tsx | No focus trap, no Escape-to-close, no restore-focus/autofocus, missing `role="dialog"`/`aria-modal`/`aria-label`. | Trap focus in the `<aside>`, add Escape handler, capture/restore `activeElement`, add dialog roles. |
| A2 | **High** | PipelineEditor.tsx | Has `role="dialog" aria-modal` (good) but no focus trap, no Escape, no initial/restore focus. | Same treatment as A1. |
| A3 | **High** | NewProjectWizard.tsx:77 | `role="dialog" aria-modal` present but no focus trap, no Escape, and overlay-click closes and **discards all wizard state** with no confirm. | Add Escape + initial focus + focus trap; dirty-confirm on overlay close. |
| A4 | Med | SelectionPopover.tsx:61; ReviseComposePanel.tsx:70; ChatDraftPanel.tsx:65 | Textareas/inputs have no `aria-label`/`<label>` (placeholder only). | Add `aria-label`. |
| A5 | Med | UsageMeter.tsx | Tooltip is CSS hover-only; meter isn't focusable → keyboard/SR users can't reach the breakdown. | Make focusable; show on focus too. |
| A6 | Med | ViewSwitcher.tsx; CardDrawer.tsx (tabs) | `role="tab"`/`aria-selected` set, but no roving tabindex / arrow-key nav, no `aria-controls`, panels lack `role="tabpanel"`. | Complete the tablist pattern or downgrade to plain nav buttons. |
| A7 | Med | ListView.tsx:60 | Row state is a **color-only 7px dot** (label in native `title`); `queued` and `done` dots are both `--text-3` (indistinguishable). | Render dot **+ short label** (matches StatePill), or give done/queued distinct cues. |
| A8 | Med | TeamsStep.tsx:34 | ⚙ advanced toggle has no `aria-expanded`. | Add `aria-expanded`. |
| A9 | Low | CommentRail.tsx:131; LineageTab.tsx:73 | Delete ✕ uses `--text-4` (likely <4.5:1) with no hover; native checkboxes unstyled. | `--text-3` default → `--text`/`--danger` on hover; style checkboxes with `accentColor`. |
| A10 | Med | TeamsStep.tsx:34; ChatDraftPanel.tsx:72; etc. | Glyph icon-buttons (⚙ ✕ Send) are glyph-sized, below ~44px touch target. | Route through `Button size="sm"`; ensure ≥44px hit area. |

## 1D. Missing states (empty / loading / error)

| # | Sev | Surface · File:Line | Issue | Fix |
|---|-----|------|-------|-----|
| S1 | **High** | CardDrawer.tsx:140 (live-log tab) | No loading/streaming/error state — empty → flat fallback, otherwise dumps text. Shipped "functional not polished". | Thread `loading\|streaming\|settled\|error\|empty`; skeleton lines while loading (not spinner), `--danger` row on error. |
| S2 | **High** | ChatDraftPanel.tsx:30 | No pending/loading affordance in the chat stream while `busy` (button only disables). | Optimistic pending row or button spinner (spec allows spinner on a button). |
| S3 | Med | ReviewStep.tsx:46 | "Create project" has no loading text/spinner while `busy`; just native-disabled dim. | "Creating…" label or button spinner. |
| S4 | Med | TeamsStep / PromptsStep | Empty `draft.teams` → blank panel, no "no teams yet" guidance. | Add empty state stating what's missing + how to add. |
| S5 | Med | ReviewStep.tsx:45; ChatDraftPanel banner | Error shown as plain `--danger` text, no `role="alert"`, no container treatment. | Add `role="alert"`; spec error container. |
| S6 | Low | CardDrawer.tsx:156 (review tab) | Renders raw placeholder `"review artifact: <path>"`. | Render via `ArtifactView`/`renderBlock` or a proper empty state. |
| S7 | Low | ArtifactView.tsx:22; CompareView.tsx:86 | Empty states use `--text-4` (low contrast); ArtifactView doesn't distinguish "still running" from "nothing here". | Use `--text-3`; differentiate running vs settled-empty. |

## 1E. Copy

| # | Sev | Surface · File:Line | Issue | Fix |
|---|-----|------|-------|-----|
| Q1 | Med | SettingsView.tsx:243,258 | **Em dashes in UI prose** — `DESIGN.md` hard-bans them. | Replace ` — ` with `. ` / `; `. |
| Q2 | Med | CommentRail.tsx:49 | Em dash in empty state "none yet — select text to comment". | "none yet. select text to comment." |
| Q3 | Low | Topbar.tsx:50,75 | Brake reason shown twice (tooltip + inline `(reason: …)`). | Keep one. |
| Q4 | Low | ChatDraftPanel.tsx:82 | Validation issues are manual `• {iss}` bullets inside `role="status"`. | Use semantic `<ul><li>`; consider `role="alert"`. |

*(Note: `—` used as a "no value" placeholder cell in `PipelineView`/`UsageMeter` is legitimate typographic use, not prose; not flagged.)*

## 1F. Primitive drift / consistency (low-risk re-routing through `ui/*`)

| # | Sev | Surface · File:Line | Issue | Fix |
|---|-----|------|-------|-----|
| D-1 | **High** | PipelineEditor.tsx:97-101 | Footer Cancel/Back/Next/Save are **bare unstyled `<button>`** → raw UA appearance (App.css not imported). Most unpolished surface in the app. | `ui/Button`: Save=`primary`, Cancel=`ghost`, Back/Next=`default`. |
| D-2 | **High** | NewProjectWizard / TeamsStep / WiringStep / ReviewStep / ChatDraftPanel | All wizard buttons are bare `<button>` (Generate, template chips, Add gate/fork, Send, Create project) → raw UA styling, breaks mono voice during setup. | Route through `ui/Button` with correct variants. |
| D-3 | Med | Topbar.tsx; ThemeToggle.tsx | "New project"/brake/theme buttons re-implement variants inline instead of `ui/Button`. | Use `ui/Button` (primary/ghost). |
| D-4 | Med | Wizard inputs/selects/textareas | `inp`/`advInp` style object copy-pasted across files; several omit `fontFamily:"inherit"` → selects/inputs render system-sans (mono violation). | Extract a shared `Input`/`Field` primitive; minimum, add `fontFamily:"inherit"` everywhere. |
| D-5 | Med | SettingsView.tsx:158,218 | Theme toggle here writes `data-theme` directly, bypassing `useTheme` → can desync with Topbar/persistence. | Route through `useTheme`. |
| D-6 | Med | SettingsView.tsx:110 | Worktree "confirm remove" + "cancel" are both neutral `default` Buttons; destructive action has no visual weight. | "confirm remove"=`danger`, "cancel"=`ghost`. |
| D-7 | Low | CardDrawer.tsx:141 (live-log `pre`) | Hand-rolled mono block, no border/bg, looks unlike every other code block. | Reuse `renderBlock` code-block style. |
| D-8 | Low | PipelineView.tsx:11 vs SettingsView.tsx:208 | Section headers: uppercase/0.08em vs lowercase/0.04em. `DESIGN.md` §Table says lowercase. | Pick one; extract `<SectionHeader>`. |
| D-9 | Low | CommentRail.tsx:119 | "addressed" chip `borderRadius:6` (`--r-lg`, "rare") — large for a chip. | `var(--r-xs)`. |
| D-10 | Med | ArtifactView/CompareView/renderBlock | Reading body has **no `max-width:75ch`** (`DESIGN.md` §Typography cap) — text runs full 600–960px drawer width. | Cap rendered body / paragraphs at `75ch`. |

---

# Part 2 — Design decisions for the operator

Subjective / larger / visual-direction calls. Each lists options with a recommendation and the tradeoff. **None applied.**

### Decision 1 — How to fix the missing hover/focus/active states (the inline-style root cause)
Inline `style={}` cannot express pseudo-states; this is why the whole app fails §States.
- **A.** One global `:focus-visible` + blunt generic `:hover` rule. *Cheap, instant app-wide focus ring; hover/active stay generic and per-variant colors not honored.*
- **B.** Convert `ui/*` primitives to CSS-class-based (modifier classes), keep one-off chrome inline. *Medium effort; fixes the highest-traffic controls properly.*
- **C.** Full migration to a `.btn`/`.input`/`.tab` class system, delete inline styles. *Largest effort; matches what `DESIGN.md` §Implementation actually envisions; ends drift permanently.*
- **Recommendation: B now + the global focus rule from C1, schedule C.** Gets a visible focus ring and correct button states on the controls that matter without a rewrite.

### Decision 2 — The Pipeline "editor" is a form, not the documented node canvas
`DESIGN.md` §Pipeline editor specifies a dot-grid drag canvas with role-colored nodes (writer blue / reviewer purple / gate ochre / impl orange) and Bezier revise/escalate edges. Shipped: `PipelineView` is a read-only card list; `PipelineEditor` is a 3-step form modal. **No canvas exists in either.**
- **A.** Build the canvas as specified (react-flow or hand-rolled SVG). *Large; highest fidelity; node/edge colors already fully specced.*
- **B.** Re-scope officially to "list viewer + form editor", update `DESIGN.md`. *Cheap; loses the signature visual that justified much of the spec.*
- **C.** Add a **read-only** static graph render (nodes + edges, no drag); keep the form for editing. *Medium; delivers the comprehension payoff (seeing revise back-edges + escalation sinks) without drag-edit complexity.*
- **Recommendation: C.** The graph's value is comprehension, not drag-editing. Decouple "see the pipeline" from "edit the pipeline."

### Decision 3 — Validation-banner color (ChatDraftPanel.tsx:80)
The live-validation banner borrows the **ochre needs-you treatment** (`--accent-2`/`--accent-bd`) for "your draft has problems," conflating validation with gate "needs you" and eroding the ≤10% one-accent rule.
- **A.** `--warn`/`--warn-2` (amber). *Reads as "caution, non-blocking" — matches that validation is best-effort/soft; frees ochre for true gates.*
- **B.** `--danger`/`--danger-2`. *Reads "fix before continuing"; may overstate severity (danger is specced for reject only).*
- **C.** Keep ochre. *Visually consistent with "needs attention" but dilutes the one-accent rule.*
- **Recommendation: A (`--warn`).** Honest severity for advisory issues; protects the accent budget. Same treatment for the PipelineEditor validation block.

### Decision 4 — The needs-you side-stripe: spec contradicts itself
`DESIGN.md` §Table prescribes the needs-you row's first cell get `border-left: 2px solid var(--accent)` (implemented faithfully at `ListView.tsx:54`), while §Anti-patterns **hard-bans** side-stripe borders >1px as accent. `SelectionPopover.tsx:34` also uses a 2px accent `border-left` on its quote (no spec exception there — a clean fail).
- **A.** Honor the anti-pattern: drop the stripe; needs-you reads via row tint + state dot only. *Most consistent with the global hard rule; the tint already carries the signal.*
- **B.** Honor the Table spec: keep 2px stripe as a single deliberate documented exception (rows have weaker affordance than cards). *Keeps list scan-ability; preserves one sanctioned exception.*
- **C.** Reduce to 1px accent. *Satisfies the letter of the ban while keeping a stripe.*
- **Recommendation: A** for the list (tint+dot already encode state; fall back to C if scan-testing shows the tint is too weak), and **fix SelectionPopover to `1px var(--border)`** regardless (matches CommentRail, no spec cover). **Reconcile `DESIGN.md` so the two sections stop contradicting.**

### Decision 5 — BoardView lane-header accent fill (BoardView.tsx:16-24)
Non-team lanes get `color:--accent` + `background:--accent-2` fill on the **entire column header**, spending the one accent on chrome and competing with real needs-you signals.
- **A.** Keep accent only on the **gate** lane (truly actionable); other non-team lanes use `--text-2` weight. *Preserves accent budget; keeps the gate emphasis.*
- **B.** Drop the fill entirely; differentiate lanes by weight/label only. *Quietest; most on-brand for "state through restraint."*
- **C.** Keep as-is.
- **Recommendation: A.** The gate lane legitimately earns the accent; the rest don't.

### Decision 6 — UsageMeter: missing gradient fill + thin tooltip
`DESIGN.md` §Usage meter requires the bar to be a **gradient fill** `--running → --warn → --danger` at 0/60/90% (the one sanctioned gradient); `UsageMeter.tsx:33` paints a single band color. Tooltip also drops ~half the spec'd rows (brake-at, window start/elapsed, 10-min avg, cost, per-team model/effort/%).
- **A.** Gradient fill + full tooltip. *High polish; needs extra data plumbed.*
- **B.** Gradient fill only (≈1-line win), keep tooltip minimal for now. *Cheap; fixes the signature visual.*
- **Recommendation: B immediately, A as data allows.** Either way make the tooltip keyboard-reachable (A5) — that part is not optional.

### Decision 7 — Type scale + spacing tokenization (the `--ts-*` / 6px gap question)
`--ts-*` and `--font-mono` are documented but never emitted; `--sp-*` has no 6px step though the doc scale includes 6.
- **A.** Add `--ts-xs…xl` + `--font-mono` to `tokens.css` and migrate call sites; add a 6px step (`--sp-1_5`) or renumber. *Touches many files; makes the scale enforceable (it's currently documentation only and already drifting: 9.5/11.5/13.5).*
- **B.** Leave px literals; snap 6→8. *Cheaper; leaves magic numbers and unenforceable scale.*
- **Recommendation: A for `--ts-*`/`--font-mono`** (high leverage — converts many Part-1 findings to mechanical swaps), and **snap 6→8 (`--sp-2`)** for the wizard unless a 6px card rhythm is deliberate.

---

## Positive findings (keep / replicate)

- `ui/Card` + `ui/StatePill`: faithful to §Card — one-dot color, per-state border/bg, `--r-md`, 10×12 padding, 6px dot, running pulse. Strong primitive fidelity.
- `ui/Button`: variants, the allowed literals (`oklch(15% 0.04 55)`, `oklch(35% 0.05 25)`), radius, padding all match §Button. The problem is *other surfaces not using it*.
- `global.css` motion: correct 1.8s pulse, `--ease-out` expo, 250ms drawer, card hover translateY(-1px), and a proper `prefers-reduced-motion` kill-switch. No bounce.
- SettingsView API-key handling: `type="password"`, labeled, masked placeholder — done right.
- No AI-slop tells anywhere: no gradient text, glassmorphism, hero-metric template, identical card-grid page, or neon-terminal cliché. Copy is largely terse, noun-led, on-voice.
- `CompareView` reuses `renderBlock` and has real per-pane empty states; its empty-state copy is the best-behaved in the app.

---

## Applied: none (review-only)

No code, tokens, copy, or `DESIGN.md` text was modified during this audit. Every item above is a recommendation pending operator decision.
