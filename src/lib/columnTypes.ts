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
