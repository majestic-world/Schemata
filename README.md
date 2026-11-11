# Schemata

Desktop database client for MySQL, MariaDB and SQLite, inspired by Navicat. Windows, Linux and macOS.

- Saved connections (passwords in the OS credential store), schema tree, object list and inspector
- Data grid with editing, table designer with DDL preview, SQL editor
- CSV/JSON/SQL export, CSV import, `.sql` file execution, database backup/restore
- Copy tables (structure and data) between databases with Ctrl+C / Ctrl+V in the tree
- Realistic test data generation per column (names, emails, addresses, lorem ipsum, dates, foreign keys…)
- Light and Navicat-inspired dark themes, switched in **Settings** (saved and restored on startup)

## Stack

Tauri 2, Rust backend, React + TypeScript frontend ([ADR 0001](docs/adr/0001-tauri-rust-react-stack.md)). Core invariants: [docs/architecture.md](docs/architecture.md).

| Path | Contents |
|---|---|
| `crates/schematadb-core` | Drivers, value model, DDL planning, import/export, dumps, connection store. No Tauri dependency. |
| `src-tauri` | Thin Tauri command layer. |
| `src` | React UI. |

## Development

Requires Rust (stable, edition 2024), Node 24 with pnpm, and Docker for the MySQL/MariaDB core tests. On Linux also `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`, plus a Secret Service (GNOME Keyring or KWallet); without one the app asks for the password when connecting.

```bash
pnpm install
pnpm tauri dev                  # run the app
pnpm typecheck && pnpm test     # frontend checks (Vitest)
cargo test -p schematadb-core   # core tests on SQLite, MySQL 8.4 and MariaDB 11.4
```

Core tests start reusable containers `schematadb-test-mysql` and `schematadb-test-mariadb`; remove them with `docker rm -f` when done. CI (`.github/workflows/ci.yml`) runs both test suites on Linux and builds installers on Windows, Ubuntu and macOS; `v*` tags also create a draft release.

## Packaging

All commands need GNU Make and Node 24. `make build` also needs pnpm, Rust and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for the current OS. Docker targets need only Docker with Linux containers and BuildKit (x86_64 emulation on ARM64 hosts); they install their own toolchains.

| Command | Environment | Output |
|---|---|---|
| `make build` | Native, current OS and architecture | `package/windows/*.exe` (NSIS), `package/linux/*.AppImage` or `package/macos/*.dmg` |
| `make linux` | Docker, Ubuntu 24.04 x86_64 | `package/linux/*.AppImage` |
| `make mac` | Docker, OSXCross | `package/macos/*_aarch64.pkg` |

Temporary build files stay in `target/package-native/`; only installers go to `package/`.

`make mac` uses the Apple SDK from [`rust-linux-darwin-builder`](https://github.com/joseluisq/rust-linux-darwin-builder), subject to Apple's terms. It produces an ad-hoc signed ARM64 `.pkg` installing into `/Applications` (macOS 11+, Apple Silicon, no universal binary). It is not a DMG because Tauri's DMG bundler needs macOS tools. Installers have no Developer ID signature or notarization, so Gatekeeper may block them; installation was not verified on a Mac.

## Limitations

- English UI only.
- No SSH tunnels, TLS options beyond driver defaults, user/privilege management, routine designer, visual view designer, or schema/data compare and sync.
- Transactional grid edits and test-data replacement require a transactional table engine.
- The designer refuses changes it cannot reproduce without losing constraints or column attributes; generated columns are read-only.
- Disconnect a connection before changing its host, user, database, engine or SQLite file.

## License

MIT, see [LICENSE](LICENSE). Third-party crates, npm packages and logos keep their own permissive licenses ([THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt), also in **Settings → About → Licenses…**). After changing dependencies, run `pnpm notices`; CI fails on `pnpm notices --check` while the file is stale. Database logos are trademarks of their owners.
