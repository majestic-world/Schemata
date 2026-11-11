# Schemata

Desktop database client for MySQL, MariaDB and SQLite, inspired by Navicat. Runs on Windows, Linux and macOS.

Features: saved connections (passwords in the OS credential store), schema tree, object list and inspector, data grid with editing, table designer with DDL preview, SQL editor, CSV/JSON/SQL export, CSV import, `.sql` file execution, database backup/restore, copying tables (structure and data) between databases with Ctrl+C / Ctrl+V in the tree, and realistic test data generation per column (names, emails, addresses, lorem ipsum, dates, foreign keys…).

Appearance: use **Settings** (the gear immediately after **Export**) to switch between the light theme and a Navicat-inspired dark theme. Changes apply immediately across panes, dialogs, the data grid and SQL editor; the choice is restored on startup.

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
make build                     # native installer for the current OS/architecture
make linux                     # Ubuntu 24.04 x86_64 AppImage, built in Docker
make mac                       # macOS Apple Silicon .pkg, cross-compiled in Docker
```

The core tests start reusable containers named `schematadb-test-mysql` and `schematadb-test-mariadb`; remove them with `docker rm -f` when no longer needed.

CI (`.github/workflows/ci.yml`) runs the core and frontend tests on Linux and builds installers on Windows, Ubuntu and macOS. Version tags (`v*`) also create a draft release.

## Packaging

GNU Make and Node 24 are required for the three `make` commands. `make build`
also needs pnpm, Rust and the [Tauri system prerequisites](https://v2.tauri.app/start/prerequisites/)
for the current OS (MSVC Build Tools on Windows, Xcode Command Line Tools on macOS,
or WebKitGTK development libraries on Linux). It builds natively, without Docker.

| Command | Build environment | Installer output |
|---|---|---|
| `make build` on Windows | Windows, current architecture | `package/windows/*.exe` (NSIS) |
| `make build` on Linux | Linux, current architecture | `package/linux/*.AppImage` |
| `make build` on macOS | macOS, current architecture | `package/macos/*.dmg` |
| `make linux` | Docker, Ubuntu 24.04 x86_64 | `package/linux/*.AppImage` |
| `make mac` | Docker, Linux x86_64 with OSXCross | `package/macos/*_aarch64.pkg` |

The Docker targets work from Windows, Linux and macOS with Docker Desktop or a
local Docker Engine configured for Linux containers and BuildKit. They install
their own toolchains; Rust, pnpm, FUSE and privileged containers are not required
on the host. ARM64 hosts need Docker's x86_64 emulation for these targets.
The first build downloads the toolchains and dependencies; subsequent builds reuse
Docker layers and Cargo caches. Temporary native build files remain under
`target/package-native/`; only installers are exported to `package/`.
The packaging scripts use Node's built-in modules, with no Python dependency
or additional npm packages.

The macOS Docker build uses the Apple SDK provided by
[`rust-linux-darwin-builder`](https://github.com/joseluisq/rust-linux-darwin-builder).
Use of that SDK is subject to Apple's applicable terms. It embeds the frontend,
creates an ARM64 `.app`, applies an ad-hoc code signature with `rcodesign`, and
packages it as a flat `.pkg` installing into `/Applications`. Requires macOS 11+
on Apple Silicon; it is not an Intel/ARM universal binary. Native `make build`
always uses the Mac's own architecture instead.

The Docker `.pkg` is intentionally not a DMG: Tauri's DMG bundler depends on
macOS tools and cannot run in a Linux container. The installers have no Developer
ID/distribution signature or notarization; Gatekeeper may block them. Generating
the cross-compiled package does not verify installation or application behavior
on a Mac.


## Limitations

English UI only. No SSH tunnels or TLS options beyond the driver's defaults, no user/privilege management, routine designer, visual view designer, schema/data compare or sync. Installers have no distribution signature or notarization.

Safety restrictions: transactional grid edits and test-data replacement require a transactional table engine. The designer refuses changes it cannot reproduce without losing existing constraints or column attributes; generated columns are read-only. Disconnect a connection before changing its host, user, database, engine or SQLite file.

## License

MIT; see [LICENSE](LICENSE). The app bundles third-party Rust crates, npm packages and logo artwork under their own (permissive) licenses; their notices are in [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) and in the app under **Settings → About → Licenses…**. After adding, removing or updating a dependency, run `pnpm notices` to regenerate the file; CI fails (`pnpm notices --check`) while it is stale. Database logos are trademarks of their respective owners.
