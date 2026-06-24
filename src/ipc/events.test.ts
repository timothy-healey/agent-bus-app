import { describe, it, expect } from "vitest";
import { EVENTS } from "./events";

/// Regression guard for the dotted-event-name bug: Tauri 2's `listen`/`emit`
/// reject any name outside `[alphanumeric, -, /, :, _]`. A `.` silently breaks
/// every subscription (no listener attaches → the UI never refreshes). This is
/// the exact charset from the runtime error message.
const TAURI_EVENT_NAME = /^[A-Za-z0-9/:_-]+$/;

describe("EVENTS contract", () => {
  for (const [key, name] of Object.entries(EVENTS)) {
    it(`${key} ("${name}") is a Tauri-legal event name`, () => {
      expect(name).toMatch(TAURI_EVENT_NAME);
      expect(name).not.toContain(".");
    });
  }
});
