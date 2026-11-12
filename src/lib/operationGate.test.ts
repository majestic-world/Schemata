import { describe, expect, it, vi } from "vitest";
import { cancelledError, operationGate } from "./operationGate";

describe("operation cancellation gate", () => {
  it("lets native calls start while not cancelled", () => {
    const gate = operationGate("op");
    expect(() => gate.checkpoint()).not.toThrow();
    expect(gate.cancelled).toBe(false);
  });

  it("keeps a cancel requested before the next native call starts, and prevents that call", async () => {
    const gate = operationGate("op");
    const runScript = vi.fn(async () => "report");
    let finishCreate!: () => void;
    const restore = (async () => {
      await new Promise<void>((resolve) => (finishCreate = resolve)); // e.g. creating the target database
      gate.checkpoint();
      return runScript();
    })();
    gate.cancel(); // the backend has nothing registered yet, so only this intent survives
    finishCreate();
    await expect(restore).rejects.toEqual(cancelledError());
    expect(runScript).not.toHaveBeenCalled();
    expect(cancelledError().kind).toBe("cancelled");
  });
});
