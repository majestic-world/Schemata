import { afterAll, afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { OBJECTS_TAB, useTabs } from "../store/tabs";
import { useWorkspace } from "../store/workspace";
import { createTreeClickNavigation } from "./treeClickNavigation";

vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  });
});

function open(name: string) {
  useTabs.getState().openTable({ connectionId: "server", database: "catalog", name, objectKind: "table" });
  return useTabs.getState().active;
}

beforeEach(() => {
  vi.useFakeTimers();
  useTabs.setState(useTabs.getInitialState(), true);
  useWorkspace.setState(useWorkspace.getInitialState(), true);
});
afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
  useTabs.setState(useTabs.getInitialState(), true);
  useWorkspace.setState(useWorkspace.getInitialState(), true);
});

afterAll(() => vi.unstubAllGlobals());

const navigation = () => createTreeClickNavigation(() => useTabs.getState().activate(OBJECTS_TAB));

describe("schema-tree click navigation", () => {
  it("opens Objects only after the native double-click interval has elapsed", async () => {
    const initial = open("first");
    await navigation().singleClick(Promise.resolve(700));
    vi.advanceTimersByTime(699);
    expect(useTabs.getState().active).toBe(initial);
    vi.advanceTimersByTime(1);
    expect(useTabs.getState().active).toBe(OBJECTS_TAB);
  });

  it("opens a double-clicked table without ever activating Objects, even with a longer system interval", async () => {
    open("first");
    const transitions: string[] = [];
    const unsubscribe = useTabs.subscribe((state, previous) => {
      if (state.active !== previous.active) transitions.push(state.active);
    });
    const clicks = navigation();
    await clicks.singleClick(Promise.resolve(1400));
    vi.advanceTimersByTime(900);
    clicks.cancel();
    const target = open("second");
    vi.advanceTimersByTime(2000);

    expect(transitions).toEqual([target]);
    expect(useTabs.getState().active).toBe(target);
    unsubscribe();
  });

  it("replaces the pending navigation when another object is clicked", async () => {
    const initial = open("first");
    const clicks = navigation();
    await clicks.singleClick(Promise.resolve(500));
    vi.advanceTimersByTime(300);
    clicks.cancel();
    await clicks.singleClick(Promise.resolve(500));
    vi.advanceTimersByTime(200);
    expect(useTabs.getState().active).toBe(initial);
    vi.advanceTimersByTime(300);
    expect(useTabs.getState().active).toBe(OBJECTS_TAB);
  });

  it("does not restore a cancelled click when native settings arrive later", async () => {
    const initial = open("first");
    let resolve!: (milliseconds: number) => void;
    const interval = new Promise<number>((done) => { resolve = done; });
    const clicks = navigation();
    const pending = clicks.singleClick(interval);
    clicks.cancel();
    resolve(500);
    await pending;
    vi.advanceTimersByTime(1000);
    expect(useTabs.getState().active).toBe(initial);
  });

  it("counts native-settings latency inside the single-click wait", async () => {
    const initial = open("first");
    let resolve!: (milliseconds: number) => void;
    const interval = new Promise<number>((done) => { resolve = done; });
    const pending = navigation().singleClick(interval);
    vi.advanceTimersByTime(250);
    resolve(500);
    await pending;
    vi.advanceTimersByTime(249);
    expect(useTabs.getState().active).toBe(initial);
    vi.advanceTimersByTime(1);
    expect(useTabs.getState().active).toBe(OBJECTS_TAB);
  });
});
