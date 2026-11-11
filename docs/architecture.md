# Architecture contracts

Invariants the core and UI rely on. The stack and module split are in [ADR 0001](adr/0001-tauri-rust-react-stack.md).

## Values

- Cells cross the Rust↔JS boundary as the tagged union `CellValue` (`crates/schematadb-core/src/value.rs`) so precision and NULL are never lost: integers beyond ±2^53 travel as `BigInt` strings, decimals as `Decimal` strings, binary as a hex preview plus length.
- Grid edits send the same union back; CSV import converts fields to text/NULL values. Both bind values as prepared-statement parameters, never interpolated. SQL exports and dumps are the exception: they serialize values as literals, since their output is SQL text.

## Errors

- One serializable `AppError { kind, message, sql_state }` crosses the command boundary. Kinds: connection, authentication, SQL, cancelled, I/O, validation. Applying a grid change set is the one exception: it fails with `{ index, error }` so the UI can point at the failing change.

## Metadata

- Object-list row counts are estimates from `information_schema` on MySQL/MariaDB and exact counts on SQLite.
- The application version is defined only in the root `Cargo.toml` (`workspace.package.version`), inherited by `src-tauri/Cargo.toml`. Tauri's config omits `version` to inherit it from Cargo; Settings reads it with `getVersion()`. The cross-platform macOS packager reads Cargo metadata exported by its build stage instead of maintaining a separate version.

## Row editing

- Row identity: primary key, else a unique key over non-null columns, else SQLite `rowid`. With none, the grid is read-only.
- The UI accumulates a change set (updates keyed by row identity, inserts, deletes); the core applies it in one transaction and reports the first failing change. On failure the UI keeps the pending changes.
- Pending edits address rows by position, so while any exist the grid keeps showing (and commits against) the page snapshot they were made on instead of refetched data. They live in a per-tab store, so switching tabs or views does not lose them.

## Table design

- A `TableDesign` value is the single input for create and alter. The core diffs the original and new designs into a DDL list; that list is both the preview the user reviews and exactly what gets executed.
- MySQL/MariaDB alterations run as `ALTER TABLE`. SQLite changes its `ALTER TABLE` cannot express use the [official rebuild procedure](https://www.sqlite.org/lang_altertable.html#otheralter): foreign keys off, new table, copy data, drop, rename, recreate indexes, triggers and dependent views, `foreign_key_check`. A failed alteration leaves the table unchanged.

## Scripts and cancellation

- The script splitter is dialect-aware (quotes, backticks, comments, `DELIMITER`, trigger bodies). In the SQL editor, execution stops at the first error; result sets are capped at 1,000 rows with continuation.
- Cancelling: MySQL/MariaDB issue `KILL QUERY <thread id>` from another connection; SQLite uses its interrupt handle. Export, import, dump and restore check a cancellation token between batches.
- Long operations stream in batches between the database and files and emit progress events; the UI never holds whole tables in memory.
- Running a `.sql` file or restoring a backup must not leave the session dirty: after an error or cancellation any open transaction is rolled back and foreign-key checks are re-enabled. The SQL editor does not do this, so deliberate transactions keep working across runs.
- Exports and backups write to a uniquely named temporary file next to the destination and replace it only on success, so a failed or cancelled run never destroys an existing file.

## Dumps

- Backups are plain `.sql` files compatible with the engine's own CLI tools. MySQL/MariaDB: foreign-key checks off during load, batched multi-row INSERTs, views after tables, triggers last. SQLite: schema SQL from `sqlite_schema` plus INSERTs in one transaction.

## Startup visibility

- The main window starts hidden. React reveals it after mounting the shell, loading fonts and allowing a render frame, without a fixed startup delay.
- The window-state plugin does not restore visibility; otherwise a previously visible window would bypass frontend readiness. Size and position are still restored, and startup remains maximized.
- Connection restoration runs in the background and does not block showing the shell.

## Toolbar icons

- Toolbar actions use filled, shaded 32 px SVG glyphs from `src/components/icons.tsx`, sharing the `Grad` helper. Settings follows the same style with a metallic gear and blue center; outline-only icons are reserved for compact controls.

## Workspace interactions

- The webview's default context menu is suppressed without stopping event propagation; app-owned Radix context menus remain available.
- Tab context menus close the selected tab, other tabs, tabs to its right, or all tabs. Objects is permanent and never included in a close operation.
- Single and grouped tab closures share `requestCloseTabs`. Unsaved SQL or pending grid edits require one confirmation before any targeted tab closes; cancelling leaves the entire group open.
- Clicking empty space or a header in the object list clears the selected object while keeping the connection and database. Toolbar actions remain outside this clearing area so they can act on the selected object.

## Persistence

- Connection definitions: JSON in the app-config directory, never containing passwords.
- Passwords: OS credential store via `keyring`, keyed by connection id. If the store is unavailable, the password is not saved and the user is prompted on connect.
- Query history: app-data directory. Window size/position: Tauri window-state plugin. Panel sizes and tree state: persisted frontend stores.
- Appearance: `schematadb.appearance` persists the light/dark choice in frontend storage. `main.tsx` applies it to the document root before React renders, so panes and portal-based dialogs share the same CSS tokens. The SQL editor also switches its CodeMirror theme for syntax highlighting.
- The app was first named ProjectCat (`dev.projectcat.app`). On startup it copies `connections.json`/`history.json` from the legacy directory when the new one lacks them; each password moves from the legacy keyring service the first time it is read, and deleting a connection removes it from both services.

## MySQL vs MariaDB

- One driver serves both, detecting the flavor from the server version. Known divergences (JSON stored as LONGTEXT on MariaDB, UUID type, metadata column differences) are why the core scenario suite runs against both servers.

## Testing

- Core tests exercise the public `schematadb-core` API against real engines (SQLite files, MySQL and MariaDB in Docker), running the same scenarios on all three. Assert resulting database state, returned values or produced files; generated SQL is verified by executing it, not by string matching.
- Frontend: Vitest on pure state modules (grid pending changes, import column mapping, etc.). No automated UI end-to-end tests; UI is validated manually.

## Pending

- The CI workflow (`.github/workflows/ci.yml`) has never run on GitHub: the Linux/macOS jobs and the `.deb`/`.AppImage`/`.dmg` artifacts are unverified until the first push.
