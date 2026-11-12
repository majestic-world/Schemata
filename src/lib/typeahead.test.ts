import { describe, expect, it } from "vitest";
import { createTypeahead, TYPEAHEAD_RESET_MS } from "./typeahead";

const names = ["encurtador", "orders", "Payments", "products", "profiles"];

describe("object list type-to-select", () => {
  it("cycles through names starting with a letter typed repeatedly", () => {
    const type = createTypeahead();
    expect(type("p", 0, names, -1)).toBe(2);
    expect(type("p", 100, names, 2)).toBe(3);
    expect(type("p", 200, names, 3)).toBe(4);
    expect(type("p", 300, names, 4)).toBe(2);
  });

  it("moves to the next match when the same letter is typed again after a pause", () => {
    const type = createTypeahead();
    expect(type("p", 0, names, -1)).toBe(2);
    expect(type("p", TYPEAHEAD_RESET_MS + 1, names, 2)).toBe(3);
  });

  it("keeps the current name while a quickly typed prefix still matches it", () => {
    const type = createTypeahead();
    expect(type("p", 0, names, 0)).toBe(2);
    expect(type("r", 50, names, 2)).toBe(3);
    expect(type("o", 100, names, 3)).toBe(3);
    expect(type("f", 150, names, 3)).toBe(4);
  });

  it("starts a new query after the pause", () => {
    const type = createTypeahead();
    expect(type("p", 0, names, -1)).toBe(2);
    expect(type("o", TYPEAHEAD_RESET_MS + 1, names, 2)).toBe(1);
  });

  it("reports no match without consuming later letters", () => {
    const type = createTypeahead();
    expect(type("x", 0, names, 1)).toBe(-1);
    expect(type("e", TYPEAHEAD_RESET_MS + 1, names, 1)).toBe(0);
  });

  it("leaves a space alone unless it continues a query", () => {
    const type = createTypeahead();
    expect(type(" ", 0, names, -1)).toBeUndefined();
    expect(type("p", 10, ["pro x", "pro y"], -1)).toBe(0);
    expect(type("r", 20, ["pro x", "pro y"], 0)).toBe(0);
    expect(type("o", 30, ["pro x", "pro y"], 0)).toBe(0);
    expect(type(" ", 40, ["pro x", "pro y"], 0)).toBe(0);
    expect(type("y", 50, ["pro x", "pro y"], 0)).toBe(1);
  });
});
