// S4: centralised selectors mirroring the real agent-bus-app UI.
// Source-of-truth references in comments so a UI rename is a one-file fix.
//
// This helper is HARNESS-LOCAL (vet F3): it holds UI selector strings only, no
// domain types, consumed solely by the e2e specs. It is NOT a published kernel
// or a cross-context contract.

export const sel = {
  // Topbar.tsx — the "New project" button.
  newProjectButton: "button=New project",
  // ProjectList.tsx — empty-state copy.
  emptyState: "*=No projects yet",
  // NewProjectWizard.tsx — the wizard dialog (role="dialog" aria-modal).
  wizardDialog: '[role="dialog"]',
  // NewProjectWizard.tsx — basics inputs (aria-labels).
  projectNameInput: "aria/Project name",
  rootPathInput: "aria/Root path",
  // ReviewStep.tsx — the final Create button.
  createProjectButton: "button=Create project",
  // NewProjectWizard.tsx — the wizard Next control.
  nextButton: "button=Next",
  // NewProjectWizard.tsx — the Template (seed) picker container heading. The
  // template buttons are the <button>s inside the div that contains this copy.
  templateButtonsXPath:
    '//div[contains(., "Or start from a template")]//button',
} as const;
