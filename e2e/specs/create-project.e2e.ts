import { $, $$, expect } from "@wdio/globals";
import os from "node:os";
import { sel } from "../helpers/selectors.js";

// The headline E2E flow, in the ubiquitous language (vet F1):
//
//   launch -> open the Design Session wizard -> fill basics -> kick off from a
//   TEMPLATE (SEED) -> step through the wizard -> CREATE-FROM-DRAFT -> see the
//   Project listed and active in the topbar.
//
// Why the Template (seed) kickoff and NOT describe->Generate (vet F2):
// the Generate path runs through the LLM Chat ACL and needs a live `claude`
// subprocess, which cannot run headless on CI. The Template (seed) path is pure
// (OHS `seed_template`, no model) so the whole flow is headless-safe. This spec
// MUST drive the seed path and MUST NOT silently fall back to Generate — if no
// template button is present it fails loudly (the assertion below), it never
// proceeds via the LLM kickoff.
describe("agent-bus-app: create a Project via the wizard (Template seed kickoff)", () => {
  const projectName = `E2E ${Date.now()}`;
  // A real, existing directory the Workspace can root the project under.
  const rootPath = os.tmpdir();

  it("kicks off from a Template seed, creates-from-draft, and lists the Project", async () => {
    // 1. Open the Design Session wizard (the only new-project path).
    await $(sel.newProjectButton).click();
    await expect($(sel.wizardDialog)).toBeDisplayed();

    // 2. Fill the Basics step.
    await $(sel.projectNameInput).setValue(projectName);
    await $(sel.rootPathInput).setValue(rootPath);

    // 3. Kick off from the FIRST bundled Template (seed) — pure OHS, no LLM.
    //    NO SILENT FALLBACK (vet F2): fail loudly if no template button exists;
    //    never proceed via the describe->Generate (live-claude) path.
    const templateButtons = await $$(sel.templateButtonsXPath);
    await expect(templateButtons.length).toBeGreaterThan(0);
    await templateButtons[0].click();

    // 4. Advance the Design Session steps: Teams -> Prompts -> Wiring -> Review.
    for (let i = 0; i < 3; i++) {
      const next = await $(sel.nextButton);
      await next.waitForClickable();
      await next.click();
    }

    // 5. Create-from-draft (hard-validate -> write -> activate).
    const create = await $(sel.createProjectButton);
    await create.waitForClickable();
    await create.click();

    // 6. The wizard closes; the Project is listed + active in the topbar.
    await expect($(sel.wizardDialog)).not.toBeDisplayed();
    await expect($(`*=${projectName}`)).toBeExisting();
  });
});
