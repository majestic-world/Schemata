import { describe, expect, it } from "vitest";
import { composeType, lengthForType, parseType, retype } from "./columnTypes";

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

describe("type picker", () => {
  it("keeps length, unsigned and zerofill when the current type is picked again in any case", () => {
    expect(retype("mySql", "decimal(12,4)", "decimal(12,4)", "DECIMAL")).toBe("DECIMAL(12,4)");
    expect(retype("mySql", "varchar(1000)", "varchar(1000)", "VARCHAR")).toBe("VARCHAR(1000)");
    expect(retype("mySql", "int unsigned zerofill", null, "INT")).toBe("INT unsigned zerofill");
    expect(retype("sqlite", "VARCHAR(80)", null, "varchar")).toBe("varchar(80)");
  });

  it("brings the new type's usual length on a real change", () => {
    expect(retype("mySql", "varchar(1000)", "varchar(1000)", "INT")).toBe("INT(11)");
    expect(retype("mySql", "int(11) unsigned zerofill", null, "BIGINT")).toBe("BIGINT(20) unsigned zerofill");
    expect(retype("mySql", "int(11) unsigned", null, "TEXT")).toBe("TEXT");
  });

  it("restores the loaded type's attributes after typing through intermediate text", () => {
    const loaded = "int(10) unsigned zerofill";
    let current = retype("mySql", loaded, loaded, "i");
    current = retype("mySql", current, loaded, "in");
    expect(retype("mySql", current, loaded, "int")).toBe("int(10) unsigned zerofill");
  });

  it("parses and composes zerofill", () => {
    expect(parseType("int(5) unsigned zerofill")).toEqual({ base: "int", length: "5", unsigned: true, zerofill: true });
    expect(composeType(parseType("int(5) zerofill"))).toBe("int(5) zerofill");
  });
});
