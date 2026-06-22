import { describe, expect, it, vi, beforeEach } from "vitest";
import { injectTopic, listTasks, approveGate, reviseGate, rejectGate, brakeOn, brakeOff, brakeState } from "./runtime";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
import { invoke } from "@tauri-apps/api/core";
const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

describe("runtime ipc", () => {
  beforeEach(() => invokeMock.mockReset());

  it("injectTopic passes topic + target_repo", async () => {
    invokeMock.mockResolvedValueOnce({ id: "T-1", state: "queued" });
    const t = await injectTopic("scheduling", "/repo");
    expect(invokeMock).toHaveBeenCalledWith("inject_topic", { topic: "scheduling", target_repo: "/repo" });
    expect(t.id).toBe("T-1");
  });

  it("listTasks calls list_tasks", async () => {
    invokeMock.mockResolvedValueOnce([]);
    await listTasks();
    expect(invokeMock).toHaveBeenCalledWith("list_tasks");
  });

  it("approveGate/reviseGate/rejectGate pass task_id", async () => {
    invokeMock.mockResolvedValue({ id: "T-1", state: "queued" });
    await approveGate("T-1");
    expect(invokeMock).toHaveBeenCalledWith("approve_gate", { task_id: "T-1" });
    await reviseGate("T-1");
    expect(invokeMock).toHaveBeenCalledWith("revise_gate", { task_id: "T-1" });
    await rejectGate("T-1");
    expect(invokeMock).toHaveBeenCalledWith("reject_gate", { task_id: "T-1" });
  });

  it("brake commands map to brake_on/off/state", async () => {
    invokeMock.mockResolvedValue({ on: true, reason: "manual" });
    await brakeOn("rate-limit");
    expect(invokeMock).toHaveBeenCalledWith("brake_on", { reason: "rate-limit" });
    await brakeOff();
    expect(invokeMock).toHaveBeenCalledWith("brake_off");
    await brakeState();
    expect(invokeMock).toHaveBeenCalledWith("brake_state");
  });
});
