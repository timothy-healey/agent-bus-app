import { describe, expect, it } from "vitest";
import { formatAge } from "./age";

describe("formatAge", () => {
  it("formats seconds", () => expect(formatAge(100, 145)).toBe("45s"));
  it("formats minutes and seconds", () => expect(formatAge(0, 263)).toBe("4m 23s"));
  it("formats hours and minutes", () => expect(formatAge(0, 3 * 3600 + 5 * 60)).toBe("3h 5m"));
  it("formats days and hours", () => expect(formatAge(0, 2 * 86400 + 3600)).toBe("2d 1h"));
  it("clamps negatives to 0s", () => expect(formatAge(200, 100)).toBe("0s"));
});
