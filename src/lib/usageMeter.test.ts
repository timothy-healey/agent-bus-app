import { describe, expect, it } from "vitest";
import { bandColorVar, brakeEta, windowLine, formatBurn, resetCountdown } from "./usageMeter";

describe("usageMeter helpers", () => {
  it("maps each band to its colour var", () => {
    expect(bandColorVar("safe")).toBe("var(--running)");
    expect(bandColorVar("warn")).toBe("var(--warn)");
    expect(bandColorVar("hot")).toBe("var(--danger)");
    expect(bandColorVar("braked")).toBe("var(--danger)");
  });

  it("formats the window line like the mockup", () => {
    expect(windowLine(18000, 1_200_000)).toBe("5h window · 1.2M tok");
    expect(windowLine(18000, 580_000)).toBe("5h window · 580k tok");
  });

  it("formats burn rate per minute", () => {
    expect(formatBurn(18_000)).toBe("18k/min");
    expect(formatBurn(2_400)).toBe("2k/min");
    expect(formatBurn(0)).toBe("0/min");
  });

  it("formats the braked reset countdown", () => {
    expect(resetCountdown(1440)).toBe("↻ 24m");
    expect(resetCountdown(90)).toBe("↻ 2m");
    expect(resetCountdown(7_320)).toBe("↻ 2h 2m");
    expect(resetCountdown(0)).toBe("↻ 0m");
  });

  it("formats brake ETA from an absolute unix timestamp and now", () => {
    // est_brake_at is 30 min ahead of now
    expect(brakeEta(1000 + 1800, 1000)).toBe("~30min");
    // 90 min ahead -> hours+min
    expect(brakeEta(1000 + 5400, 1000)).toBe("~1h 30min");
  });
  it("returns null when est_brake_at is null", () => {
    expect(brakeEta(null, 1000)).toBeNull();
  });
  it("returns null when the projected time is already past", () => {
    expect(brakeEta(900, 1000)).toBeNull();
  });
});
