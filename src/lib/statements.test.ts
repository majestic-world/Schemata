import { describe, expect, it } from "vitest";
import { isReadOnlyQuery, statementAt } from "./statements";

/** Script with a `|` marking the cursor. */
const at = (marked: string, engine: "mySql" | "sqlite" = "mySql") => {
  const cursor = marked.indexOf("|");
  return statementAt(marked.slice(0, cursor) + marked.slice(cursor + 1), cursor, engine)?.script ?? null;
};

describe("statement at the cursor", () => {
  it("runs only the statement the cursor is in", () => {
    expect(at("SELECT 1;\nDELETE FROM t;\nSELECT |2;")).toBe("SELECT 2");
    expect(at("SELECT 1;\nDELETE| FROM t;\nSELECT 2;")).toBe("DELETE FROM t");
    expect(at("SELECT 1;|\nDELETE FROM t;")).toBe("SELECT 1");
    expect(at("SELECT 1;\n|DELETE FROM t")).toBe("DELETE FROM t");
  });

  it("counts a comment after a statement on its line, but runs nothing from blank lines between statements", () => {
    expect(at("SELECT 1; -- one|\nDELETE FROM t;")).toBe("SELECT 1");
    expect(at("SELECT 1;\n\n|\nDELETE FROM t;")).toBe(null);
    expect(at("SELECT 1;\n-- note|")).toBe(null);
    expect(at("SELECT 1;\n-- about the delete|\nDELETE FROM t;")).toBe(null);
    expect(at("SELECT 1;\n  |  DELETE FROM t;")).toBe("DELETE FROM t");
    expect(at("|")).toBe(null);
  });

  it("does not split inside quotes or comments", () => {
    expect(at("UPDATE t SET a = 'x;|y' WHERE id = 1;\nDROP TABLE t;")).toBe("UPDATE t SET a = 'x;y' WHERE id = 1");
    expect(at("SELECT 'it\\'s;' AS a|;\nDROP TABLE t;")).toBe("SELECT 'it\\'s;' AS a");
    expect(at("SELECT /* ; */ 1|;\nDROP TABLE t;")).toBe("SELECT /* ; */ 1");
  });

  it("keeps a MySQL DELIMITER block together and re-declares its delimiter", () => {
    const script = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN DELETE FROM t; |SELECT 1; END$$\nDELIMITER ;\nDROP TABLE t;";
    expect(at(script)).toBe("DELIMITER $$\nCREATE PROCEDURE p() BEGIN DELETE FROM t; SELECT 1; END\n$$\n");
    expect(at("DELIMITER $$\nSELECT 1$$\nDELIMITER ;\nDROP |TABLE t;")).toBe("DROP TABLE t");
  });

  it("keeps a SQLite trigger body together instead of running one of its statements", () => {
    const script = "CREATE TRIGGER tr AFTER INSERT ON a BEGIN DELETE FROM b; |UPDATE c SET x = 1; END;\nDROP TABLE a;";
    expect(at(script, "sqlite")).toBe("CREATE TRIGGER tr AFTER INSERT ON a BEGIN DELETE FROM b; UPDATE c SET x = 1; END");
  });

  it("reports the trimmed range of the statement", () => {
    expect(statementAt("SELECT 1;\n  SELECT 2  ;", 14, "mySql")).toEqual({ from: 12, to: 20, script: "SELECT 2" });
  });
});

describe("read-only statements", () => {
  it("accepts plain reads", () => {
    for (const sql of ["SELECT * FROM t", "  -- c\n/* x */ (SELECT 1)", "with x as (select 1) select * from x", "SHOW TABLES", "explain select 1"]) {
      expect(isReadOnlyQuery(sql), sql).toBe(true);
    }
  });

  it("refuses writes and anything else", () => {
    for (const sql of ["UPDATE t SET a = 1", "SELECT * FROM t FOR UPDATE", "SELECT 1 INTO @x", "WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d", "CALL p()", "SELECT nextval(s)", "SET @a = 1", ""]) {
      expect(isReadOnlyQuery(sql), sql).toBe(false);
    }
  });
});
