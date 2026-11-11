import type { Engine } from "../api/types";

const MYSQL_TYPES = [
  "INT", "BIGINT", "SMALLINT", "TINYINT", "MEDIUMINT", "DECIMAL", "FLOAT", "DOUBLE", "BIT", "BOOLEAN",
  "CHAR", "VARCHAR", "TINYTEXT", "TEXT", "MEDIUMTEXT", "LONGTEXT", "BINARY", "VARBINARY", "TINYBLOB", "BLOB",
  "MEDIUMBLOB", "LONGBLOB", "DATE", "DATETIME", "TIMESTAMP", "TIME", "YEAR", "JSON", "ENUM", "SET",
];
const MARIADB_EXTRA = ["UUID", "INET4", "INET6"];
const SQLITE_TYPES = ["INTEGER", "REAL", "TEXT", "BLOB", "NUMERIC", "BOOLEAN", "DATE", "DATETIME", "VARCHAR", "DECIMAL"];

export function typeSuggestions(engine: Engine): string[] {
  switch (engine) {
    case "mySql":
      return MYSQL_TYPES;
    case "mariaDb":
      return [...MYSQL_TYPES, ...MARIADB_EXTRA];
    case "sqlite":
      return SQLITE_TYPES;
    default: {
      const never: never = engine;
      return never;
    }
  }
}

export const FK_ACTIONS = ["NO ACTION", "RESTRICT", "CASCADE", "SET NULL", "SET DEFAULT"];
export const MYSQL_ENGINES = ["InnoDB", "MyISAM", "MEMORY", "ARCHIVE", "CSV"];
export const MARIADB_ENGINES = ["InnoDB", "Aria", "MyISAM", "MEMORY", "ARCHIVE", "CSV"];

/** Splits `decimal(12,4) unsigned` into its parts; `compose` reverses it. */
export function parseType(dataType: string): { base: string; length: string; unsigned: boolean } {
  const m = /^\s*([a-zA-Z][\w ]*?)?\s*(?:\((.*)\))?\s*(unsigned)?\s*(zerofill)?\s*$/i.exec(dataType);
  if (!m) return { base: dataType, length: "", unsigned: false };
  return { base: m[1] ?? "", length: m[2] ?? "", unsigned: !!m[3] };
}

export function composeType(base: string, length: string, unsigned: boolean): string {
  return `${base.trim()}${length.trim() ? `(${length.trim()})` : ""}${unsigned ? " unsigned" : ""}`;
}

export const isNumericType = (base: string) => /int|decimal|numeric|float|double|real|bit/i.test(base);

/** Length Navicat fills in when a column switches to a type; types absent here take none. */
const MYSQL_LENGTHS: Record<string, string> = {
  TINYINT: "4", SMALLINT: "6", MEDIUMINT: "9", INT: "11", INTEGER: "11", BIGINT: "20",
  DECIMAL: "10,2", NUMERIC: "10,2", BIT: "1",
  CHAR: "255", VARCHAR: "255", BINARY: "255", VARBINARY: "255",
};
/** SQLite only enforces names, and `INTEGER` must stay bare to remain the rowid alias. */
const SQLITE_LENGTHS: Record<string, string> = { VARCHAR: "255", CHAR: "255", DECIMAL: "10,2", NUMERIC: "10,2" };

/**
 * Length for a column whose type was changed to `base`: the type's default for a type this engine
 * knows, otherwise `current` (the user typed something custom).
 */
export function lengthForType(engine: Engine, base: string, current: string): string {
  const upper = base.trim().toUpperCase();
  if (!typeSuggestions(engine).includes(upper) && !(upper in MYSQL_LENGTHS && engine !== "sqlite")) return current;
  return (engine === "sqlite" ? SQLITE_LENGTHS : MYSQL_LENGTHS)[upper] ?? "";
}
