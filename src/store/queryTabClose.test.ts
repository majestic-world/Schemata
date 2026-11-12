import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { closeTabsFor, useTabs } from "./tabs";
import { useQueryResults } from "./queryResults";

const { closeEditorSession } = vi.hoisted(() => ({ closeEditorSession: vi.fn((_id: string, _session: string) => Promise.resolve()) }));
vi.mock("../api/client", () => ({ api: { closeEditorSession } }));

function reset() {
  useTabs.setState(useTabs.getInitialState(), true);
  useQueryResults.setState(useQueryResults.getInitialState(), true);
  closeEditorSession.mockClear();
}

beforeEach(reset);
afterEach(reset);

function runQuery(connectionId: string) {
  useTabs.getState().newQuery({ connectionId, database: "catalog" });
  const id = useTabs.getState().active;
  useQueryResults.getState().set(id, {
    connectionId, database: "catalog", result: null, failure: null, running: true, executionId: "e1", startedAt: 0, finishedAt: null,
  });
  return id;
}

describe("closing a query tab", () => {
  it("removes its run, ignores late results and ends its editor session", () => {
    const id = runQuery("server");
    useTabs.getState().close(id);

    expect(id in useQueryResults.getState().runs).toBe(false);
    useQueryResults.getState().patch(id, { running: false }, "e1");
    expect(id in useQueryResults.getState().runs).toBe(false);
    expect(closeEditorSession).toHaveBeenCalledWith("server", id);
  });

  it("ends the editor sessions of query tabs closed with their connection", () => {
    const first = runQuery("server");
    const other = runQuery("other");
    closeTabsFor("server");

    expect(closeEditorSession.mock.calls).toEqual([["server", first]]);
    expect(Object.keys(useQueryResults.getState().runs)).toEqual([other]);
  });
});
