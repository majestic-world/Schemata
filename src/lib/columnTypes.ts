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

export type ParsedType = { base: string; length: string; unsigned: boolean; zerofill: boolean };

/** Splits `decimal(12,4) unsigned zerofill` into its parts; `composeType` reverses it. */
export function parseType(dataType: string): ParsedType {
  const m = /^\s*([a-zA-Z][\w ]*?)?\s*(?:\((.*)\))?\s*(unsigned)?\s*(zerofill)?\s*$/i.exec(dataType);
  if (!m) return { base: dataType, length: "", unsigned: false, zerofill: false };
  return { base: m[1] ?? "", length: m[2] ?? "", unsigned: !!m[3], zerofill: !!m[4] };
}

export function composeType({ base, length, unsigned, zerofill }: ParsedType): string {
  return `${base.trim()}${length.trim() ? `(${length.trim()})` : ""}${unsigned ? " unsigned" : ""}${zerofill ? " zerofill" : ""}`;
}

const sameBase = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/**
 * Data type of a column whose type picker now reads `base`. Picking the column's current base type (any case)
 * keeps its length, unsigned and zerofill; going back to the base type of `original` (the column's type when the
 * design was loaded) restores its attributes, so text typed on the way (`i`, `in`) loses nothing. Text that is
 * not a type the engine knows keeps the attributes too; only a real change of type brings the type's usual
 * length (like Navicat: INT → 11, DECIMAL → 10,2).
 */
export function retype(engine: Engine, current: string, original: string | null, base: string): string {
  const t = parseType(current);
  if (sameBase(base, t.base)) return composeType({ ...t, base });
  const o = original === null ? null : parseType(original);
  if (o && sameBase(base, o.base)) return composeType({ ...o, base });
  const length = lengthForType(engine, base, t.length);
  if (!isKnownType(engine, base)) return composeType({ ...t, base, length });
  const numeric = isNumericType(base);
  return composeType({ base, length, unsigned: t.unsigned && numeric, zerofill: t.zerofill && numeric });
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
  if (!isKnownType(engine, base)) return current;
  return (engine === "sqlite" ? SQLITE_LENGTHS : MYSQL_LENGTHS)[base.trim().toUpperCase()] ?? "";
}

function isKnownType(engine: Engine, base: string): boolean {
  const upper = base.trim().toUpperCase();
  return typeSuggestions(engine).includes(upper) || (upper in MYSQL_LENGTHS && engine !== "sqlite");
}
