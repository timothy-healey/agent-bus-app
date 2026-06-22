import { describe, expect, it } from "vitest";
import { costBand, formatTokens } from "./cost";

describe("costBand", () => {
  it("green below 20k", () => {
    expect(costBand(0)).toBe("green");
    expect(costBand(19_999)).toBe("green");
  });
  it("amber from 20k to 100k inclusive of lower bound", () => {
    expect(costBand(20_000)).toBe("amber");
    expect(costBand(100_000)).toBe("amber");
  });
  it("red above 100k", () => {
    expect(costBand(100_001)).toBe("red");
    expect(costBand(154_000)).toBe("red");
  });
});

describe("formatTokens", () => {
  it("formats thousands with a k suffix", () => {
    expect(formatTokens(47_000)).toBe("47k");
    expect(formatTokens(1_200_000)).toBe("1.2M");
  });
  it("shows raw for small counts", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
  });
});
