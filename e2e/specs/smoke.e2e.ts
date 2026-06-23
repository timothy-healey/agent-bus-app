import { $, expect } from "@wdio/globals";
import { sel } from "../helpers/selectors.js";

// Smoke: the app boots and renders the empty project state. Fails fast if the
// binary didn't launch or the webview didn't mount.
describe("agent-bus-app: smoke", () => {
  it("launches and shows the empty project state", async () => {
    const newProject = await $(sel.newProjectButton);
    await expect(newProject).toBeDisplayed();

    const empty = await $(sel.emptyState);
    await expect(empty).toBeExisting();
  });
});
