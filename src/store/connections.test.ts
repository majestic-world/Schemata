import { describe, expect, it } from "vitest";
import type { ConnectionConfig } from "../api/types";
import { connectionTargetChanged } from "./connections";

const original: ConnectionConfig = {
  id: "connection",
  name: "Original",
  engine: "mySql",
  color: null,
  host: "localhost",
  port: 3306,
  user: "root",
  database: null,
  filePath: null,
  savePassword: true,
};

describe("connection destination locking", () => {
  it("allows cosmetic and credential preferences to change without replacing the target", () => {
    expect(connectionTargetChanged(original, {
      ...original,
      name: "Renamed",
      color: "#14a3a3",
      savePassword: false,
    })).toBe(false);
  });

  it.each<Partial<ConnectionConfig>>([
    { engine: "mariaDb" },
    { host: "another-host" },
    { port: 3307 },
    { user: "another-user" },
    { database: "another-database" },
    { filePath: "another.db" },
  ])("detects a destination change: %o", (change) => {
    expect(connectionTargetChanged(original, { ...original, ...change })).toBe(true);
  });

  it("detects changing the SQLite file", () => {
    const sqlite: ConnectionConfig = {
      ...original,
      engine: "sqlite",
      host: null,
      port: null,
      user: null,
      filePath: "original.db",
      savePassword: false,
    };
    expect(connectionTargetChanged(sqlite, { ...sqlite, filePath: "different.db" })).toBe(true);
    expect(connectionTargetChanged(sqlite, { ...sqlite, name: "Renamed" })).toBe(false);
  });
});
