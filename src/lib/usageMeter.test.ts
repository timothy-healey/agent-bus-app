import { describe, expect, it } from "vitest";
import { bandColorVar, resetCountdown, isStale, asOf, formatUsd } from "./usageMeter";

describe("usageMeter helpers", () => {
  it("maps each band to its colour var", () => {
    expect(bandColorVar("safe")).toBe("var(--running)");
    expect(bandColorVar("warn")).toBe("var(--warn)");
    expect(bandColorVar("hot")).toBe("var(--danger)");
    expect(bandColorVar("braked")).toBe("var(--danger)");
  });

  it("formats the braked reset countdown", () => {
    expect(resetCountdown(1440)).toBe("↻ 24m");
    expect(resetCountdown(90)).toBe("↻ 2m");
    expect(resetCountdown(7_320)).toBe("↻ 2h 2m");
    expect(resetCountdown(0)).toBe("↻ 0m");
  });
});

describe("isStale", () => {
  it("is false within 3 minutes and true after", () => {
    expect(isStale(1_000, 1_000 + 180)).toBe(false);
    expect(isStale(1_000, 1_000 + 181)).toBe(true);
  });
  it("is true when there has never been a reading", () => {
    expect(isStale(null, 1_000)).toBe(true);
  });
});

describe("asOf", () => {
  it("formats the reading time as HH:MM local", () => {
    const d = new Date(2026, 9, 7, 14, 32);
    expect(asOf(Math.floor(d.getTime() / 1000))).toBe("as of 14:32");
  });
});

describe("formatUsd", () => {
  it("shows cents, and <$0.01 for tiny non-zero cost", () => {
    expect(formatUsd(0.25)).toBe("$0.25");
    expect(formatUsd(0.004)).toBe("<$0.01");
    expect(formatUsd(0)).toBe("$0.00");
  });
});
