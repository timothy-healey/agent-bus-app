import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

import {
  DEFAULT_TEAM_MODEL,
  getModelList,
  modelLabel,
  refreshModelList,
  sourceLabel,
  supportsEffort,
  type ModelList,
} from "./models";

const LIST: ModelList = {
  source: "live",
  cli_version: "2.1.292",
  models: [
    { value: "opus", resolved_model: "claude-opus-5-5", display_name: "Opus 5.5", description: null, effort_levels: ["low", "high", "xhigh"], supports_auto_mode: true },
    { value: "claude-opus-4-6", resolved_model: "claude-opus-4-6", display_name: "Opus 4.6", description: null, effort_levels: ["low", "high"], supports_auto_mode: true },
    { value: "haiku", resolved_model: "claude-haiku-4-5-20251001", display_name: "Haiku 4.5", description: null, effort_levels: [], supports_auto_mode: false },
  ],
};

describe("model list helpers", () => {
  it("labels an alias with what it resolves to and a pinned id by itself", () => {
    expect(modelLabel(LIST.models[0])).toBe("opus → claude-opus-5-5");
    expect(modelLabel(LIST.models[1])).toBe("claude-opus-4-6");
    expect(modelLabel({ ...LIST.models[1], resolved_model: null })).toBe("claude-opus-4-6");
  });

  it("shows the curated source as built-in", () => {
    expect(sourceLabel("curated")).toBe("built-in");
    expect(sourceLabel("cached")).toBe("cached");
    expect(sourceLabel("live")).toBe("live");
  });

  it("supportsEffort reads the model's levels", () => {
    expect(supportsEffort(LIST, "opus", "xhigh")).toBe(true);
    expect(supportsEffort(LIST, "claude-opus-4-6", "xhigh")).toBe(false);
    expect(supportsEffort(LIST, "missing", "low")).toBe(false);
  });

  it("new teams default to the default alias", () => {
    expect(DEFAULT_TEAM_MODEL).toBe("default");
  });
});

describe("model list ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("getModelList calls model_list", async () => {
    invokeMock.mockResolvedValueOnce(LIST);
    expect(await getModelList()).toEqual(LIST);
    expect(invokeMock).toHaveBeenCalledWith("model_list");
  });

  it("refreshModelList calls refresh_model_list", async () => {
    invokeMock.mockResolvedValueOnce(LIST);
    expect(await refreshModelList()).toEqual(LIST);
    expect(invokeMock).toHaveBeenCalledWith("refresh_model_list");
  });
});
