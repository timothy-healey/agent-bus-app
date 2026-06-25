import type { Page } from "@playwright/test";
import type { MockState } from "../mock/backend";

/// Seed the mock backend BEFORE the app bundle loads. `addInitScript` runs in the
/// page context before any module, so we stash the payload on `window.__E2E_SEED__`;
/// the mock module reads it on load and applies it via `seed(...)`. (Seeding after
/// mount via `window.__E2E__.seed` is also possible but would race the app's first
/// data fetches — pre-mount is the deterministic path.)
export async function seedBeforeMount(page: Page, state: Partial<MockState>): Promise<void> {
  await page.addInitScript((s) => {
    (window as unknown as { __E2E_SEED__?: unknown }).__E2E_SEED__ = s;
  }, state);
}

/// Fire an event into the mock event bus from the test (drives `task-changed` /
/// `run-changed` refreshes). Requires the app to be mounted (`window.__E2E__`).
export async function emitEvent(page: Page, event: string, payload: unknown): Promise<void> {
  await page.evaluate(
    ([e, p]) => window.__E2E__?.emit(e as string, p),
    [event, payload] as const,
  );
}

/// Read the live mock state back (assert handler mutations, e.g. retry requeued).
export async function readState(page: Page): Promise<MockState | undefined> {
  return page.evaluate(() => window.__E2E__?.getState());
}
