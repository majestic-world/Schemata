import { describe, expect, it } from "vitest";
import { assign, autoMap, toMapping } from "./importMapping";

describe("CSV column mapping", () => {
  it("matches headers to columns ignoring case and punctuation, skipping unknown ones", () => {
    expect(autoMap(["ID", "First Name", "extra", "e-mail"], ["id", "first_name", "email"])).toEqual(["id", "first_name", null, "email"]);
  });

  it("never maps two CSV columns to the same table column", () => {
    expect(autoMap(["name", "Name"], ["name"])).toEqual(["name", null]);
    expect(assign(["id", "name"], 1, "id")).toEqual([null, "id"]);
  });

  it("produces source-index mappings only for assigned columns", () => {
    expect(toMapping([null, "id", null, "name"])).toEqual([
      { source: 1, target: "id" },
      { source: 3, target: "name" },
    ]);
  });
});
