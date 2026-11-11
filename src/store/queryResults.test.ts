import { afterEach, describe, expect, it } from "vitest";
import { useQueryResults, type QueryRun } from "./queryResults";

const firstRun: QueryRun = {
  connectionId: "original-connection",
  database: "original-database",
  executionId: "first-execution",
  result: null,
  failure: null,
  running: true,
  startedAt: 1,
  finishedAt: null,
};

afterEach(() => useQueryResults.setState(useQueryResults.getInitialState(), true));

describe("query execution context", () => {
  it("retains the execution destination when the tab selection changes and results arrive", () => {
    const tab = { connectionId: firstRun.connectionId, database: firstRun.database };
    useQueryResults.getState().set("query", { ...firstRun, ...tab });
    tab.connectionId = "different-connection";
    tab.database = "different-database";
    useQueryResults.getState().patch("query", {
      result: { outcomes: [], error: null },
      running: false,
      finishedAt: 2,
    }, firstRun.executionId);

    expect(useQueryResults.getState().runs.query).toMatchObject({
      connectionId: "original-connection",
      database: "original-database",
      executionId: "first-execution",
      running: false,
    });
  });

  it("keeps an absent execution database absent when the tab later selects one", () => {
    useQueryResults.getState().set("query", { ...firstRun, database: null });
    useQueryResults.getState().patch("query", { running: false, finishedAt: 2 }, firstRun.executionId);
    expect(useQueryResults.getState().runs.query?.database).toBeNull();
  });

  it("ignores late continuation results from an earlier execution", () => {
    const nextRun = { ...firstRun, executionId: "second-execution", database: "next-database" };
    useQueryResults.getState().set("query", nextRun);
    useQueryResults.getState().patch("query", {
      result: { outcomes: [], error: null },
      running: false,
    }, firstRun.executionId);

    expect(useQueryResults.getState().runs.query).toBe(nextRun);
  });

  it("does not recreate a closed tab when an execution finishes", () => {
    useQueryResults.getState().set("query", firstRun);
    useQueryResults.getState().set("query", undefined);
    useQueryResults.getState().patch("query", { running: false }, firstRun.executionId);
    expect(useQueryResults.getState().runs.query).toBeUndefined();
  });
});
