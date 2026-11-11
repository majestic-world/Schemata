# SchemataDb

Desktop database client for MySQL, MariaDB and SQLite, inspired by Navicat. Runs on Windows, Linux and macOS.

Features: saved connections (passwords in the OS credential store), schema tree, object list and inspector, data grid with editing, table designer with DDL preview, SQL editor, CSV/JSON/SQL export, CSV import, `.sql` file execution, database backup/restore, copying tables (structure and data) between databases with Ctrl+C / Ctrl+V in the tree, and realistic test data generation per column (names, emails, addresses, lorem ipsum, dates, foreign keys…).

## Stack

Tauri 2 shell, Rust backend, React + TypeScript frontend (see [ADR 0001](docs/adr/0001-tauri-rust-react-stack.md)). Core invariants (value model, row identity, DDL planning, cancellation, persistence, testing) are in [docs/architecture.md](docs/architecture.md).

| Path | Contents |
|---|---|
| `crates/schematadb-core` | Drivers, value model, DDL planning, import/export, dumps, connection store. No Tauri dependency. |
| `src-tauri` | Thin Tauri command layer. |
| `src` | React UI. |

## Requirements

- Rust (stable, edition 2024) and Node 24 with pnpm.
- Linux: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`, and a Secret Service (GNOME Keyring or KWallet) to store passwords. Without one, the app asks for the password when connecting.
- Docker, to run the MySQL/MariaDB core tests.

## Commands

```bash
pnpm install
pnpm tauri dev                  # run the app
pnpm typecheck && pnpm test     # frontend checks (Vitest)
cargo test -p schematadb-core   # core tests on SQLite, MySQL 8.4 and MariaDB 11.4
pnpm tauri build                # installers in target/release/bundle
```

The core tests start reusable containers named `schematadb-test-mysql` and `schematadb-test-mariadb`; remove them with `docker rm -f` when no longer needed.

CI (`.github/workflows/ci.yml`) runs the core and frontend tests on Linux and builds installers on Windows, Ubuntu and macOS. Version tags (`v*`) also create a draft release.

## Limitations

Light theme and English UI only. No SSH tunnels or TLS options beyond the driver's defaults, no user/privilege management, routine designer, visual view designer, schema/data compare or sync, and installers are not code-signed.
