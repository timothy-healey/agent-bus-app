import { describe, expect, it, vi, beforeEach } from "vitest";
import { listPlugins, pluginLabel, type PluginInfo } from "./plugins";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("plugins ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("listPlugins calls list_plugins", async () => {
    const rows: PluginInfo[] = [{ name: "superpowers", marketplace: "official", path: "/p/superpowers" }];
    invokeMock.mockResolvedValueOnce(rows);
    expect(await listPlugins()).toEqual(rows);
    expect(invokeMock).toHaveBeenCalledWith("list_plugins");
  });

  it("pluginLabel names the marketplace", () => {
    expect(pluginLabel({ name: "ddd-council", marketplace: "ddd-council", path: "/x" })).toBe("ddd-council (ddd-council)");
  });
});
