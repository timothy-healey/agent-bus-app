import { describe, expect, it, vi, beforeEach } from "vitest";
import { injectTopic, listTasks, approveGate, reviseGate, rejectGate, brakeOn, brakeOff, brakeState, startRun, listRuns, runStoreOccupancy, listInvocations, retryTask, forceAdvance, abandonTask, acceptTask, type Run, type StoreOccupancy, type InvocationRow } from "./runtime";

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

  it("startRun calls start_run with an optional topic (null when omitted)", async () => {
    const run: Run = { id: "R-1", pipeline: "p", project_id: "proj", generator_dry: false, completed: false, created_at: 7 };
    invokeMock.mockResolvedValueOnce(run);
    const r = await startRun();
    expect(invokeMock).toHaveBeenCalledWith("start_run", { topic: null });
    // the Run wire shape round-trips intact
    expect(r).toEqual(run);
    invokeMock.mockResolvedValueOnce(run);
    await startRun("context");
    expect(invokeMock).toHaveBeenCalledWith("start_run", { topic: "context" });
  });

  it("listRuns calls list_runs with project_id and returns Run[]", async () => {
    const runs: Run[] = [
      { id: "R-2", pipeline: "p", project_id: "proj", generator_dry: true, completed: false, created_at: 9 },
    ];
    invokeMock.mockResolvedValueOnce(runs);
    const got = await listRuns("proj");
    expect(invokeMock).toHaveBeenCalledWith("list_runs", { project_id: "proj" });
    expect(got).toEqual(runs);
  });

  it("runStoreOccupancy calls run_store_occupancy and returns StoreOccupancy[]", async () => {
    const occ: StoreOccupancy[] = [{ stage: "spec", occupancy: 2, capacity: 3 }];
    invokeMock.mockResolvedValueOnce(occ);
    const got = await runStoreOccupancy("R-1");
    expect(invokeMock).toHaveBeenCalledWith("run_store_occupancy", { run_id: "R-1" });
    expect(got).toEqual(occ);
  });

  it("listInvocations calls list_invocations and returns InvocationRow[] intact", async () => {
    const rows: InvocationRow[] = [
      {
        invocation_id: "I-1", team_id: "spec", model: "claude-opus-4-8", attempts: 3,
        started_at: 100, settled_at: 110, outcome: "error:model_unavailable",
        input_tokens: 50, output_tokens: 12,
      },
    ];
    invokeMock.mockResolvedValueOnce(rows);
    const got = await listInvocations("T-1");
    expect(invokeMock).toHaveBeenCalledWith("list_invocations", { task_id: "T-1" });
    // the wire shape round-trips intact (wire-contract).
    expect(got).toEqual(rows);
  });

  it("retry/forceAdvance/abandon/accept pass task_id to their commands", async () => {
    invokeMock.mockResolvedValue({ id: "T-1", state: "queued" });
    await retryTask("T-1");
    expect(invokeMock).toHaveBeenCalledWith("retry_task", { task_id: "T-1" });
    await forceAdvance("T-1");
    expect(invokeMock).toHaveBeenCalledWith("force_advance", { task_id: "T-1" });
    await abandonTask("T-1");
    expect(invokeMock).toHaveBeenCalledWith("abandon_task", { task_id: "T-1" });
    await acceptTask("T-1");
    expect(invokeMock).toHaveBeenCalledWith("accept_task", { task_id: "T-1" });
  });
});
