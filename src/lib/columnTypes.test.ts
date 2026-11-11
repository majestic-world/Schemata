import { describe, expect, it } from "vitest";
import { lengthForType } from "./columnTypes";

describe("length for a changed column type", () => {
  it("fills the usual MySQL/MariaDB lengths and clears lengths a type does not take", () => {
    expect(lengthForType("mySql", "int", "255")).toBe("11");
    expect(lengthForType("mariaDb", "DECIMAL", "255")).toBe("10,2");
    expect(lengthForType("mySql", "varchar", "")).toBe("255");
    expect(lengthForType("mySql", "TEXT", "255")).toBe("");
    expect(lengthForType("mySql", "datetime", "11")).toBe("");
  });

  it("keeps SQLite INTEGER bare so it stays the rowid alias", () => {
    expect(lengthForType("sqlite", "INTEGER", "255")).toBe("");
    expect(lengthForType("sqlite", "VARCHAR", "")).toBe("255");
  });

  it("leaves the length alone while the type is not one the engine knows (e.g. still being typed)", () => {
    expect(lengthForType("mySql", "varc", "100")).toBe("100");
    expect(lengthForType("sqlite", "INT", "")).toBe("");
  });
});
