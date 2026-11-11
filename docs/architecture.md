# Architecture contracts

Invariants the core and UI rely on. The stack and module split are in [ADR 0001](adr/0001-tauri-rust-react-stack.md).

## Values

- Cells cross the Rust↔JS boundary as the tagged union `CellValue` (`crates/schematadb-core/src/value.rs`) so precision and NULL are never lost: integers beyond ±2^53 travel as `BigInt` strings, decimals as `Decimal` strings, binary as a hex preview plus length.
- Grid edits send the same union back; CSV import converts fields to text/NULL values. Both bind values as prepared-statement parameters, never interpolated. SQL exports and dumps are the exception: they serialize values as literals, since their output is SQL text.

## Errors

- One serializable `AppError { kind, message, sql_state }` crosses the command boundary. Kinds: connection, authentication, SQL, cancelled, I/O, validation. Applying a grid change set is the one exception: it fails with `{ index, error }` so the UI can point at the failing change.

## Metadata

- Object-list row counts are estimates from `information_schema` on MySQL/MariaDB and exact counts on SQLite. On MySQL 8+, list and table-info reads disable the session's `information_schema_stats_expiry` cache on the acquired connection, so refresh reads current engine statistics instead of values cached for up to a day. Counts remain estimates, avoiding full-table scans.
- The application version is defined only in the root `Cargo.toml` (`workspace.package.version`), inherited by `src-tauri/Cargo.toml`. Tauri's config omits `version` to inherit it from Cargo; Settings reads it with `getVersion()`. The cross-platform macOS packager reads Cargo metadata exported by its build stage instead of maintaining a separate version.

## Row editing

- Row identity: primary key, else a complete nonpartial unique key over non-null named columns, else an unshadowed SQLite rowid alias, else (MySQL/MariaDB tables without a key, like Navicat) every column whose value matches exactly. By-value changes compare with `<=>` so NULL matches NULL and add `LIMIT 1`, so of several identical rows only one changes. FLOAT, binary, BIT, JSON and spatial columns are left out of the match (previews or approximate storage); with nothing left, and for views, the grid is read-only.
- The UI accumulates a change set (updates keyed by row identity, inserts, deletes); the core applies it in one transaction and reports the first failing change. On failure the UI keeps the pending changes.
- Pending edits address rows by position, so while any exist the grid keeps showing (and commits against) the page snapshot they were made on instead of refetched data. They live in a per-tab store, so switching tabs or views does not lose them. Grid mutations are blocked while committing; saving finalizes the active cell before collecting the change set.

## Table design

- A `TableDesign` value is the single input for create and alter. The core diffs the original and new designs into a DDL list; that list is both the preview the user reviews and exactly what gets executed.
- MySQL/MariaDB alterations run as `ALTER TABLE`. SQLite changes its `ALTER TABLE` cannot express use the [official rebuild procedure](https://www.sqlite.org/lang_altertable.html#otheralter): foreign keys off, new table, copy data, drop, rename, recreate indexes, triggers and dependent views, `foreign_key_check`. A failed alteration leaves the table unchanged.
- MySQL/MariaDB DDL names tables without the database (``CREATE TABLE `t` ``); `apply_ddl` runs `USE <db>` on its connection first. SQLite DDL stays schema-qualified, since SQLite has no `USE` and attached databases need the prefix.
- Schema changes must not silently remove attributes the designer cannot represent. Unsupported SQLite rebuilds and MySQL column redefinitions are rejected before executing DDL. SQLite rebuilds preserve the AUTOINCREMENT high-water mark.
- Generated columns remain visible in the data grid but are not writable and are excluded from backup/export INSERTs. SQLite row metadata and selected values use the same column ordering.
- A new table is named in the review dialog when saving (button or Ctrl+S), not in the designer; until then the SQL preview tab uses the placeholder `new_table`.

## Scripts and cancellation

- The script splitter is dialect-aware (quotes, backticks, comments, `DELIMITER`, trigger bodies). In the SQL editor, execution stops at the first error; result sets are capped at 1,000 rows with continuation.
- Cancelling: MySQL/MariaDB issue `KILL QUERY <thread id>` through a control connection independent of the work pool; SQLite uses its interrupt handle. File execution also interrupts the active statement, not only the next batch.
- Long operations stream in batches between the database and files and emit progress events; the UI never holds whole tables in memory. SQLite table copies stage rows in a temporary file before importing, avoiding a reader/writer deadlock on the shared connection.
- Running a `.sql` file or restoring a backup must not leave the session dirty: after an error or cancellation any open transaction is rolled back and foreign-key checks are re-enabled. The SQL editor does not do this, so deliberate transactions keep working across runs.
- Exports and backups write to a uniquely named temporary file next to the destination and replace it only on success, so a failed or cancelled run never destroys an existing file.
- SQL files require valid UTF-8 (with an optional BOM); invalid bytes fail the operation instead of silently replacing stored text.
- Replacing existing rows during test-data generation uses DELETE and INSERT within the same transaction. Failed planning, insertion or cancellation must preserve the previous data. Operations requiring rollback reject nontransactional table engines.

## Dumps

- Backups are plain `.sql` files compatible with the engine's own CLI tools. MySQL/MariaDB: foreign-key checks off during load, batched multi-row INSERTs, views after tables, triggers last. SQLite: schema SQL from `sqlite_schema` plus INSERTs in one transaction.

## Startup visibility

- The main window starts hidden. React reveals it after mounting the shell, loading fonts and allowing a render frame, without a fixed startup delay.
- The window-state plugin does not restore visibility; otherwise a previously visible window would bypass frontend readiness. Size and position are still restored, and startup remains maximized.
- Connection restoration runs in the background and does not block showing the shell.

## Toolbar icons

- Toolbar actions use filled, shaded 32 px SVG glyphs from `src/components/icons.tsx`, sharing the `Grad` helper. Settings follows the same style with a metallic gear and blue center; outline-only icons are reserved for compact controls.

## Control styling

- Shared `.btn` hover changes the background without replacing the variant's border color. Primary buttons keep their accent border, ghost buttons stay borderless, and neutral buttons retain their normal frame in both themes.
- Modals use `--dialog-border` instead of the strong input border: the light theme keeps its existing frame, while dark dialogs use a subdued charcoal outline without changing field borders.

## Workspace interactions

- On Windows, startup disables WebView2's default context menus with `SetAreDefaultContextMenusEnabled(false)` for the entire webview, independently of DOM event propagation. The frontend context-menu listener remains a fallback for other platforms. Browser-owned menus are disabled; app-owned Radix menus still receive DOM events.
- Windows startup also disables WebView2's browser shortcuts (`SetAreBrowserAcceleratorKeysEnabled(false)`: Ctrl+N new window, Ctrl+F find, Ctrl+P print, zoom, DevTools keys). WebView2 consumed Ctrl+N before the page saw it; with the setting off, key events reach the frontend, which owns every shortcut.
- Ctrl+N opens the table designer for a new table in the selected database (same as the toolbar's Table button). It ignores key repeat and open dialogs and reports when no database is selected.
- Tables and views in the schema tree and central object list share `ObjectContextMenu`. Opening it selects the clicked object; actions use that object's explicit reference. Menu portals do not count as clicks on the list's empty background.
- F6 opens the existing rename dialog for the selected table while Objects is active. It ignores key repeat, modifier chords, text editing, open dialogs/menus, and non-table selections.
- Tab context menus close the selected tab, other tabs, tabs to its right, or all tabs. Objects is permanent and never included in a close operation.
- Single and grouped tab closures share `requestCloseTabs`. Unsaved SQL or pending grid edits require one confirmation before any targeted tab closes; cancelling leaves the entire group open.
- Table-designer drafts survive tab and view switches and participate in the same discard confirmation. Native window closure (including Alt+F4) also checks unsaved work and active operations.
- Query results retain the connection/database context of execution for continuation and export; changing the editor's database does not change existing results.
- Changing a saved connection's target requires disconnecting first, including while its initial connection is being established.
- Clicking empty space or a header in the object list clears all selected objects while keeping the connection and database. Toolbar actions remain outside this clearing area so they can act on the selection.
- Right-clicking blank space in Objects opens database-scoped actions and clears object selection. New Table opens the designer; Execute Sql File opens the SQL runner; Dump Sql File and Dump Sql With Data reuse the backup dialog with structure-only and structure-plus-data presets. Table rows retain their own context menus, including table-specific data generation.
- Execute SQL File separates General configuration from Message Log results. The log shows attempted/successful/failed statement counts from the final engine report, elapsed time and byte-read progress; counts remain unknown while running or when no report is available. Only a successful report turns the bar green. UTF-8/BOM decoding, continue-on-error and script-controlled transactions retain existing execution semantics.
- In Objects, Ctrl+A selects the currently visible, search-filtered objects; Ctrl-click toggles individual items. Delete, the toolbar and the context menu share one confirmed bulk-drop action. If a drop fails, processing stops, successful drops are refreshed, and failed/unattempted objects remain selected.
- In data grids, Ctrl+A selects the loaded page, including virtualized rows outside the viewport, never unloaded pages. Delete stages those rows through the normal pending-change flow; Commit applies them. Text editors retain native Ctrl+A/Delete behavior, and read-only grids cannot stage deletions.
- Null values and empty-row placeholders reserve 2 px inside their text clipping box for italic glyph overhang. Column clipping and ellipsis remain enabled for long values.
- Successful object-list and data-grid refetches briefly fade the existing table and restore it, even when its rows have not changed. F5/Ctrl+R and the refresh button share this feedback without remounting rows or clearing selection. The pulse lasts 240 ms, reduced to 120 ms with a gentler opacity change when reduced motion is preferred; initial loads, failed fetches and switching to another page, sort, filter or database do not pulse.
- The tree footer filters the schema tree, independently of the toolbar's object search: its search matches database names of loaded connections; the filter popover keeps only open connections ("Active Object") and/or chosen engines; the star shows only favorite tables/views and the connections and databases that contain them. Locate expands, selects and scrolls to the active tab's object (or the current selection), lifting tree filters that would hide it; collapse all clears the expanded-node state.
- The toolbar's "Search objects" (Ctrl+K) filters only the central object list; it never hides tree nodes.

## Persistence

- Connection definitions: JSON in the app-config directory, never containing passwords.
- Passwords: OS credential store via `keyring`, keyed by connection id. If the store is unavailable, the password is not saved and the user is prompted on connect. Connections with password saving disabled never read an old stored credential; failures to remove credentials are reported.
- Query history: app-data directory, atomically replaced; damaged history is preserved without preventing startup. A process-lifetime profile lock prevents concurrent application instances from overwriting saved state. Window size/position: Tauri window-state plugin. Panel sizes and tree state: persisted frontend stores.
- Favorites (`schematadb.favorites`) and tree filters (`schematadb.treeFilter`, without the search text) persist in frontend storage. Favorites follow table renames and are dropped with the object.
- The Create table dialog's "Add `created_at` and `updated_at`" choice (`schematadb.designerPrefs`, off by default) persists and applies to later new tables until changed; it appends two nullable `DATETIME` columns, skipping names the design already has.
- Appearance: `schematadb.appearance` persists the light/dark choice in frontend storage. `main.tsx` applies it to the document root before React renders, so panes and portal-based dialogs share the same CSS tokens. The SQL editor also switches its CodeMirror theme for syntax highlighting.
- The app was first named ProjectCat (`dev.projectcat.app`). On startup it copies `connections.json`/`history.json` from the legacy directory when the new one lacks them; each password moves from the legacy keyring service the first time it is read, and deleting a connection removes it from both services.

## MySQL vs MariaDB

- One driver serves both, detecting the flavor from the server version. Known divergences (JSON stored as LONGTEXT on MariaDB, UUID type, metadata column differences) are why the core scenario suite runs against both servers.

## Testing

- Core tests exercise the public `schematadb-core` API against real engines (SQLite files, MySQL and MariaDB in Docker), running the same scenarios on all three. Assert resulting database state, returned values or produced files; generated SQL is verified by executing it, not by string matching.
- Frontend: Vitest on pure state modules (grid pending changes, import column mapping, etc.). No automated UI end-to-end tests; UI is validated manually.

## Release-candidate validation (2026-11-11)

- Rust workspace: 207 tests passed, including real SQLite, MySQL 8.4 and MariaDB 11.4 scenarios; workspace Clippy passed with warnings denied.
- Frontend: production build/typecheck, 52 Vitest tests and third-party license verification passed. Production npm dependency audit found no known vulnerabilities.
- Real Windows UI smoke used an isolated application profile and disposable databases: Ctrl+S with only the active cell and with an existing pending edit, generated-column read-only behavior, designer drafts across tabs/views, cancellation of native close, locked connection targets, and continuation of a 1,001-row query after changing the editor's database.
- Docker packaging produced `package/linux/Schemata_0.1.0_amd64.AppImage` and `package/macos/Schemata_0.1.0_aarch64.pkg`. These are build proofs, not installation/runtime validation on those operating systems.
- Native Windows packaging produced `package/windows/Schemata_0.1.0_x64-setup.exe` with the final fixes. Installer execution was not performed; the real application UI was exercised separately in an isolated release-mode profile.
- The approved native-close path requires both `core:window:allow-close` and `core:window:allow-destroy`: Tauri's `onCloseRequested` implementation calls `destroy()` after an event is accepted. Windows closure smoke verified that clicking the caption X removed the isolated application's window and terminated its process with exit code 0; merely dismissing the confirmation is not sufficient proof.

## Pending

- The GitHub CI workflow has not been exercised here. Native Linux/macOS CI jobs, `.deb`/`.dmg` output, and installation/runtime smoke tests on Linux and macOS remain unverified.
- Dependency audit on 2026-11-11: RustSec reports soundness warnings for Linux transitive dependencies `glib 0.18.5` (`RUSTSEC-2024-0429`, through Tauri/GTK) and `memmap2 0.5.10` (`RUSTSEC-2026-0186`, through decorum/linicon). Their patched major/minor versions are not drop-in replacements for the upstream dependency requirements. These warnings are not a clean Linux security approval. Five additional unmaintained-crate warnings remain; the audit reports no vulnerability errors.
