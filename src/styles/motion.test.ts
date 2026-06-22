import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const tokens = readFileSync(resolve(here, "tokens.css"), "utf8");
const global = readFileSync(resolve(here, "global.css"), "utf8");

describe("motion tokens + reduced-motion", () => {
  it("defines the ease + duration tokens from DESIGN.md", () => {
    expect(tokens).toContain("--ease-out: cubic-bezier(0.16, 1, 0.3, 1)");
    expect(tokens).toContain("--dur-fast: 100ms");
    expect(tokens).toContain("--dur-base: 150ms");
    expect(tokens).toContain("--dur-slow: 250ms");
  });

  it("defines a pulse keyframe and a reduced-motion guard", () => {
    expect(global).toContain("@keyframes abp-pulse");
    expect(global).toContain("@media (prefers-reduced-motion: reduce)");
    expect(global).toContain("animation: none !important");
  });
});
