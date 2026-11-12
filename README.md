# Schemata

Desktop database client for MySQL, MariaDB and SQLite, inspired by Navicat. Windows, Linux and macOS.

- Saved connections (passwords in the OS credential store), schema tree, object list and inspector
- Data grid with editing, table designer with DDL preview, SQL editor
- CSV/JSON/SQL export, CSV import, `.sql` file execution, database backup/restore (one file, or one file per table)
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

All commands need GNU Make and Node 24. `make build` also needs pnpm, Rust and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for the current OS. On Windows, install Inno Setup 6.6 or newer; the build finds `ISCC.exe` in `PATH` or in `C:\Program Files (x86)\Inno Setup 6`. Docker targets need only Docker with Linux containers and BuildKit (x86_64 emulation on ARM64 hosts); they install their own toolchains.

| Command | Environment | Output |
|---|---|---|
| `make build` | Native, current OS and architecture | `package/windows/*.exe` (Inno Setup), `package/linux/*.AppImage` or `package/macos/*.dmg` |
| `make linux` | Docker, Ubuntu 24.04 x86_64 | `package/linux/*.AppImage` |
| `make mac` | Docker, OSXCross | `package/macos/*_aarch64.pkg` |

Temporary build files stay in `target/package-native/`; only installers go to `package/`.

Windows uses the root `setup.iss`, with `ProjectDir` resolved from the script location (`{#ProjectDir}` is an Inno preprocessor value, not a runtime constant). The installer reads its version from the Cargo-built executable and includes `LICENSE` and `THIRD_PARTY_LICENSES.txt`; the frontend, SQLite and application icon are embedded in the executable. `src-tauri/icons/icon.ico` contains multiple sizes converted from `designs/icons/app-icon-1024.png` and is also the setup icon. The installer supports English and Brazilian Portuguese, Start Menu and optional desktop shortcuts, and uninstall without deleting saved user data. If WebView2 is missing, setup downloads and installs Microsoft's Evergreen bootstrapper before installing the app; this requires internet access. After a native build, `ISCC.exe setup.iss` can recompile only the installer, from any working directory when given the script's full path. The first Inno installation does not automatically migrate an older NSIS installation; uninstall the NSIS version first (preserving user data).

`make mac` uses the Apple SDK from [`rust-linux-darwin-builder`](https://github.com/joseluisq/rust-linux-darwin-builder), subject to Apple's terms. It produces an ad-hoc signed ARM64 `.pkg` installing into `/Applications` (macOS 11+, Apple Silicon, no universal binary). It is not a DMG because Tauri's DMG bundler needs macOS tools. Installers have no Developer ID signature or notarization, so Gatekeeper may block them; installation was not verified on a Mac.

## Limitations

- English UI only.
- No SSH tunnels, TLS options beyond driver defaults, user/privilege management, routine designer, visual view designer, or schema/data compare and sync.
- Transactional grid edits and test-data replacement require a transactional table engine.
- The designer refuses changes it cannot reproduce without losing constraints or column attributes; generated columns are read-only.
- Disconnect a connection before changing its host, user, database, engine or SQLite file.

## App data

On Windows, each installation stores its files in `%APPDATA%\schemata\<hash>`, where `<hash>` is the 32-character hexadecimal MD5 of the canonical executable directory (UTF-8, lowercased on Windows). Connections, query history, saved queries, window state and WebView2 preferences share that root; passwords stay in the OS credential store under an installation-specific namespace. Open the active profile from **Settings → App data → Open folder**.

Different installation directories are isolated, including normal development and installed builds. Updates in the same directory retain data; moving the app starts a new profile. Old data directories are never imported automatically. To reset, close the app, delete its hashed profile, then reopen: it starts clean. This does not delete any databases outside that profile or old credentials from the OS vault.

## License

MIT, see [LICENSE](LICENSE). Third-party crates, npm packages and logos keep their own permissive licenses ([THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt), also in **Settings → About → Licenses…**). After changing dependencies, run `pnpm notices`; CI fails on `pnpm notices --check` while the file is stale. Database logos are trademarks of their owners.
