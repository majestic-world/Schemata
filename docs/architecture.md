# Architecture contracts

Invariants the core and UI rely on. The stack and module split are in [ADR 0001](adr/0001-tauri-rust-react-stack.md).

## Values

- Cells cross the Rust↔JS boundary as the tagged union `CellValue` (`crates/schematadb-core/src/value.rs`) so precision and NULL are never lost: integers beyond ±2^53 travel as `BigInt` strings, decimals as `Decimal` strings, binary as a hex preview plus length.
- Grid edits send the same union back; CSV import converts fields to text/NULL values (`0x…` hex into binary columns and numbers into BIT columns, as exports write them). Both bind values as prepared-statement parameters, never interpolated. SQL exports and dumps are the exception: they serialize values as literals, since their output is SQL text; each multi-row INSERT stays under 1 MB (or holds a single row).

## Grid edits

- A change set carries the row identity the grid loaded; the backend refuses it when the table's identity changed meanwhile (primary key moved, designer change). Keyed statements must hit exactly one row, or the transaction rolls back.
- Tables without a key match rows by value with `LIMIT 1`; when some columns cannot be compared (binary, JSON, FLOAT, BIT, spatial) and two rows match, the change is refused instead of hitting either row. SQLite never matches by value: it prefers the rowid over unique keys, and rowid changes also compare the loaded row values, so a reused or renumbered rowid is never written. SQLite table rebuilds keep rowids.
- Pool connections (grid, DDL, imports) run with `STRICT_ALL_TABLES` and without `NO_BACKSLASH_ESCAPES`, so bad input fails instead of being converted. Query tabs on SQLite use their own connection, like MySQL/MariaDB, so app operations never join a query tab's transaction. Run SQL File on MySQL/MariaDB uses a dedicated connection, so a script's session settings never reach the pool.
- The cell editor is a textarea: line breaks survive, and opening and closing it without typing stages nothing.
- The table designer refuses to plan against a table that changed since it was loaded, applies only DDL that re-planning still reproduces exactly, and discards pending grid edits (after confirmation) before altering a table.

## Errors

- One serializable `AppError { kind, message, sql_state }` crosses the command boundary. Kinds: connection, authentication, SQL, cancelled, I/O, validation. Applying a grid change set is the one exception: it fails with `{ index, error }` so the UI can point at the failing change.

## Metadata

- Object-list row counts are estimates from `information_schema` on MySQL/MariaDB and exact counts on SQLite. On MySQL 8+, list and table-info reads disable the session's `information_schema_stats_expiry` cache on the acquired connection, so refresh reads current engine statistics instead of values cached for up to a day. Counts remain estimates, avoiding full-table scans.
- The application version is defined only in the root `Cargo.toml` (`workspace.package.version`), inherited by `src-tauri/Cargo.toml`. Tauri's config omits `version` to inherit it from Cargo; Settings reads it with `getVersion()`. The cross-platform macOS packager reads Cargo metadata exported by its build stage instead of maintaining a separate version.
- Release candidates use SemVer pre-release versions (`1.0.0-rc.1`). MSI only accepts numeric pre-release identifiers and RPM rejects `-` in the version, so Windows ships the NSIS setup only (`make`) and Linux builds the AppImage only.

## Row editing

- Row identity: primary key, else a complete nonpartial unique key over non-null named columns, else an unshadowed SQLite rowid alias, else (MySQL/MariaDB tables without a key, like Navicat) every column whose value matches exactly. MySQL functional indexes (expression parts) never identify rows. By-value changes compare with `<=>` so NULL matches NULL and add `LIMIT 1`, so of several identical rows only one changes; textual columns compare as UTF-8 bytes (`CAST(CONVERT(col USING utf8mb4) AS BINARY)`), so case-insensitive or PAD SPACE collations cannot match a different row. FLOAT, binary, BIT, JSON and spatial columns are left out of the match (previews or approximate storage); with nothing left, and for views, the grid is read-only.
- The UI accumulates a change set (updates keyed by row identity, inserts, deletes); the core applies it in one transaction and reports the first failing change. On failure the UI keeps the pending changes.
- Pending edits address rows by position, so while any exist the grid keeps showing (and commits against) the page snapshot they were made on instead of refetched data. They live in a per-tab store, so switching tabs or views does not lose them. Grid mutations are blocked while committing; saving finalizes the active cell before collecting the change set. Pending column maps read and write own properties only, so columns named like `constructor` or `__proto__` behave as ordinary columns.
- Date-picker confirmation accepts the selected value before restoring grid focus. Otherwise blur can finalize the previous input text and suppress the pick; Cancel returns to the editor without applying the calendar selection.
- Enter (or a Shift+double-click that keeps the selection) on a cell inside a multi-cell selection saves the typed text to every selected cell; a plain click edits that cell only. Generated, binary and deleted-row cells keep their values, and each cell compares the text with its own value, so cells already holding it stay unchanged. All targets are staged as one pending change.

## Table design

- A `TableDesign` value is the single input for create and alter. The core diffs the original and new designs into a DDL list; that list is both the preview the user reviews and exactly what gets executed.
- MySQL/MariaDB alterations run as `ALTER TABLE`. SQLite changes its `ALTER TABLE` cannot express use the [official rebuild procedure](https://www.sqlite.org/lang_altertable.html#otheralter): foreign keys off, new table, copy data, drop, rename, recreate indexes, triggers and dependent views, `foreign_key_check`. A failed alteration leaves the table unchanged.
- MySQL/MariaDB DDL names tables without the database (``CREATE TABLE `t` ``); `apply_ddl` runs `USE <db>` on its connection first. SQLite DDL stays schema-qualified, since SQLite has no `USE` and attached databases need the prefix.
- Schema changes must not silently remove attributes the designer cannot represent. Unsupported SQLite rebuilds and MySQL column redefinitions are rejected before executing DDL. SQLite rebuilds preserve the AUTOINCREMENT high-water mark. A SQLite rebuild that would keep no original column (dropping every row) is rejected, as is one that renames the table or a kept column while triggers or views depend on it; a rename saved on its own uses `ALTER TABLE … RENAME`, which rewrites dependents.
- SQLite foreign keys declared without target columns (`REFERENCES parent`) resolve to the parent's primary key.
- Generated columns remain visible in the data grid but are not writable and are excluded from backup/export INSERTs. SQLite row metadata and selected values use the same column ordering.
- Moving or deleting designer columns, indexes or foreign keys first commits buffered (unblurred) inputs, which are keyed by row position.
- A new table is named in the review dialog when saving (button or Ctrl+S), not in the designer; until then the SQL preview tab uses the placeholder `new_table`.

## Scripts and cancellation

- The script splitter is dialect-aware (quotes, backticks, comments, `DELIMITER`, trigger bodies). In the SQL editor, execution stops at the first error; result sets are capped at 1,000 rows with continuation.
- Cancelling: MySQL/MariaDB issue `KILL QUERY <thread id>` through a control connection independent of the work pool; SQLite uses its interrupt handle. File execution also interrupts the active statement, not only the next batch. A cancel that arrives before a long operation registers is kept, so the operation starts cancelled; multi-step dialog flows (create database then restore, prepare then copy) check the cancel between steps, and a dismissed dialog never starts its pending step.
- Long operations stream in batches between the database and files and emit progress events; the UI never holds whole tables in memory. SQLite table copies stage rows in a temporary file before importing, avoiding a reader/writer deadlock on the shared connection.
- Copying a table within one engine recreates it from its own definition (`SHOW CREATE TABLE` / `sqlite_schema`), renaming database-scoped constraint and index names after the copy, so ON UPDATE, column collations, CHECK, STRICT, WITHOUT ROWID, generated, INVISIBLE and partial/expression-index definitions survive; generated values are recomputed, not copied. Copies into another engine translate the table design and refuse generated columns. Copies between MySQL/MariaDB servers read and write TIMESTAMPs in UTC.
- Running a `.sql` file or restoring a backup must not leave the session dirty: after an error or cancellation any open transaction is rolled back and foreign-key checks are re-enabled. The SQL editor does not do this, so deliberate transactions keep working across runs: each MySQL/MariaDB query tab owns a dedicated server session outside the pool, keyed by the tab id, so transactions, user variables and temporary tables persist across runs and continuation. Closing the tab or disconnecting ends the session, and the server rolls back its open transaction. A session the server closed (timeout, KILL, restart) fails its next run with a connection error instead of reconnecting silently and hiding the rollback; the following run opens a fresh session.
- Exports and backups write to a uniquely named temporary file next to the destination and replace it only on success, so a failed or cancelled run never destroys an existing file.
- SQL files are decoded as UTF-8 (a BOM always means UTF-8) or Windows-1252. The whole file is decoded before the first statement runs, so a decoding error never leaves earlier statements applied; invalid bytes fail the operation instead of silently replacing stored text. CSV import is equally strict: invalid UTF-8 fails with a hint to choose Windows-1252, and rows whose field count differs from the header (or first row) are row failures. Windows-1252 exports fail on characters the encoding lacks.
- CSV exports write NULL as `\N` by default and imports read that marker back as NULL, so NULL and the empty string stay distinct. Exporting a query result runs the query again, so only read-only queries can be exported.
- Replacing existing rows during test-data generation uses DELETE and INSERT within the same transaction. Failed planning, insertion or cancellation must preserve the previous data. Operations requiring rollback reject nontransactional table engines. Sequences that would exceed the 64-bit integer range are rejected before inserting.
- MySQL/MariaDB imports retry a failed multi-row INSERT row by row only for statement-level errors. Deadlocks (1213) and lock wait timeouts (1205) can roll back the whole transaction, so they abort the import instead of letting later rows autocommit.
- Exports that include `CREATE TABLE` under another destination name replace only the statement's table-name token with the quoted destination; other statement shapes fail instead of writing broken SQL.
- The SQL editor saves files atomically (temporary file in the same directory, then replace). Opening a file requires UTF-8 and strips a leading BOM.

## Dumps

- Backups are plain `.sql` files compatible with the engine's own CLI tools. MySQL/MariaDB: foreign-key checks off during load, batched multi-row INSERTs, views after tables, triggers last; every table is read on one session inside one `START TRANSACTION WITH CONSISTENT SNAPSHOT`, with TIMESTAMPs in UTC, and that session is closed afterwards even on error or cancel. SQLite: schema SQL from `sqlite_schema` plus INSERTs in one transaction.
- "One file per table" in the backup dialog (also Objects ▸ … ▸ Export Each Table to SQL…) writes `<table>.sql` per table into a chosen folder, read the same way (one snapshot). Each file restores on its own: prologue, `DROP`/`CREATE`, rows, then the table's own indexes, triggers and SQLite autoincrement counter; views, routines and events are left out. File names replace characters Windows forbids, suffix device names (`CON`, `NUL`, …) and number names equal ignoring case. Same-named files are replaced only after every table is written. SQLite dumps restore autoincrement counters per table (`DELETE … WHERE name = …` then `INSERT`), never clearing other tables' counters.
- Restoring over an existing database requires typing its name. The file is checked first; `USE` and `CREATE DATABASE` statements are listed in the confirmation, since statements after them run in that database. "Delete existing rows first" in data generation confirms, naming every table whose foreign keys reference the target.

## Startup visibility

- The main window starts hidden. React reveals it after mounting the shell, loading fonts and allowing a render frame, without a fixed startup delay.
- The window-state plugin does not restore visibility; otherwise a previously visible window would bypass frontend readiness. Size and position are still restored, and startup remains maximized.
- Connection restoration runs in the background and does not block showing the shell.
- A second launch from the same installation directory exits through `tauri-plugin-single-instance` and brings that installation's running window forward. Different installation directories have independent identities and locks and can run concurrently.

## Toolbar icons

- Toolbar actions use filled, shaded 32 px SVG glyphs from `src/components/icons.tsx`, sharing the `Grad` helper. Settings follows the same style with a metallic gear and blue center; outline-only icons are reserved for compact controls.
- The MySQL silhouette in `src/components/engineLogos.ts` uses continuous Bézier curves at the tail/body join, without lower-edge notches or a thin trailing extension. The tail tip is slightly rounded to remain clean at small icon sizes.

## Control styling

- Shared `.btn` hover changes the background without replacing the variant's border color. Primary buttons keep their accent border, ghost buttons stay borderless, and neutral buttons retain their normal frame in both themes.
- Modals use `--dialog-border` instead of the strong input border: the light theme keeps its existing frame, while dark dialogs use a subdued charcoal outline without changing field borders.
- Settings includes an “App data” section below About, with an “Open folder” button. A parameterless backend command opens the running installation's hashed profile in the OS file manager. The frontend cannot supply arbitrary paths.

## Workspace interactions

- On Windows, startup disables WebView2's default context menus with `SetAreDefaultContextMenusEnabled(false)` for the entire webview, independently of DOM event propagation. The frontend context-menu listener remains a fallback for other platforms. Browser-owned menus are disabled; app-owned Radix menus still receive DOM events.
- Windows startup also disables WebView2's browser shortcuts (`SetAreBrowserAcceleratorKeysEnabled(false)`: Ctrl+N new window, Ctrl+F find, Ctrl+P print, zoom, DevTools keys). WebView2 consumed Ctrl+N before the page saw it; with the setting off, key events reach the frontend, which owns every shortcut.
- Ctrl+N opens the table designer for a new table in the selected database (same as the toolbar's Table button) and reports when no database is selected. Ctrl+T, Ctrl+N and Ctrl+D ignore key repeat, already-handled events and open dialogs, and blur the focused editor before switching tab or view, so an active grid cell or designer input commits instead of unmounting with its text.
- Tables and views in the schema tree and central object list share `ObjectContextMenu`. Opening it selects the clicked object; actions use that object's explicit reference. Menu portals do not count as clicks on the list's empty background.
- Table menus offer Empty Table (`DELETE`: keeps the auto-increment counter; referencing foreign keys apply their ON DELETE actions on every engine) and Truncate Table (resets the counter: MySQL/MariaDB `TRUNCATE`, which refuses tables other tables reference; SQLite deletes the rows and the table's `sqlite_sequence` entry in one transaction). Both confirm, describing what referencing tables will do.
- F6 opens the existing rename dialog for the selected table while Objects is active. It ignores key repeat, modifier chords, text editing, open dialogs/menus, and non-table selections.
- Tab context menus close the selected tab, other tabs, tabs to its right, or all tabs. Objects is permanent and never included in a close operation.
- Single and grouped tab closures share `requestCloseTabs`. Unsaved SQL or pending grid edits require one confirmation before any targeted tab closes; cancelling leaves the entire group open.
- Table-designer drafts survive tab and view switches and participate in the same discard confirmation. Native window closure (including Alt+F4) also checks unsaved work and active operations.
- Opening or activating a table/view tab selects that exact connection, database and object in the schema tree and inspector. Closing the active tab selects its surviving table tab; closing a background tab or returning to Objects does not overwrite a separate browsing selection. Tree rows expose the same state through `aria-selected`.
- A single tree-object click selects immediately and opens Objects only after the double-click interval (the Windows system setting; 500 ms on other platforms). A double click opens the table/view directly without an intermediate Objects activation. A new pointer/keyboard interaction, tab change or sidebar unmount cancels pending single-click navigation, including while the native setting is still being read.
- Query results retain the connection/database context of execution for continuation and export; changing the editor's database does not change existing results. Closing a query tab drops its results.
- CSV import enables Import only for a preview of the current file and options; changing either clears the preview and mapping, and responses to outdated preview requests are ignored.
- Changing a saved connection's target requires disconnecting first, including while its initial connection is being established.
- Clicking empty space or a header in the object list clears all selected objects while keeping the connection and database. Toolbar actions remain outside this clearing area so they can act on the selection.
- Right-clicking blank space in Objects opens database-scoped actions and clears object selection. New Table opens the designer; Execute Sql File opens the SQL runner; Dump Sql File and Dump Sql With Data reuse the backup dialog with structure-only and structure-plus-data presets. Table rows retain their own context menus, including table-specific data generation.
- Execute SQL File separates General configuration from Message Log results. The log shows attempted/successful/failed statement counts from the final engine report, elapsed time and byte-read progress; counts remain unknown while running or when no report is available. Only a successful report turns the bar green; a transaction the script left open is reported as rolled back, not as success. The file is read as UTF-8 or Windows-1252; continue-on-error and script-controlled transactions retain existing execution semantics.
- In Objects, Ctrl+A selects the currently visible, search-filtered objects; Ctrl-click toggles individual items. Delete, the toolbar and the context menu share one confirmed bulk-drop action. If a drop fails, processing stops, successful drops are refreshed, and failed/unattempted objects remain selected.
- In data grids, Ctrl+A selects the loaded page, including virtualized rows outside the viewport, never unloaded pages. Delete stages those rows through the normal pending-change flow; Commit applies them. Text editors retain native Ctrl+A/Delete behavior, and read-only grids cannot stage deletions. Elsewhere Ctrl+A does nothing: the app blocks the webview's select-all of the whole page outside inputs, textareas and content-editable editors.
- All grid values reserve 2 px inside their text clipping box so final glyphs (including digits and italic placeholders) are not cropped by glyph overhang or fractional-pixel rasterization. Column clipping and ellipsis remain enabled for long values.
- In data grids and query results, Ctrl+F or F3 in the grid opens a "Find column" field at the end of the toolbar (data grids) or result footer (query results); the grid never shows it otherwise. It matches column names case-insensitively as substrings, scrolls the match to the middle of the data area and highlights its header and cells; Enter/F3 and Shift+Enter/Shift+F3 step through matches, Esc closes it and returns focus to the grid. The shortcut is ignored while a cell editor is open, so it never saves an edit in progress. Without a vertical scrollbar, the mouse wheel scrolls the grid horizontally.
- Successful object-list and data-grid refetches briefly fade the existing table and restore it, even when its rows have not changed. F5/Ctrl+R and the refresh button share this feedback without remounting rows or clearing selection. The pulse lasts 240 ms, reduced to 120 ms with a gentler opacity change when reduced motion is preferred; initial loads, failed fetches and switching to another page, sort, filter or database do not pulse.
- The tree footer filters the schema tree, independently of the toolbar's object search: its search matches database names of loaded connections; the filter popover keeps only open connections ("Active Object") and/or chosen engines; the star shows only favorite tables/views and the connections and databases that contain them. Locate expands, selects and scrolls to the active tab's object (or the current selection), lifting tree filters that would hide it; collapse all clears the expanded-node state.
- The toolbar's "Search objects" (Ctrl+K) filters only the central object list; it never hides tree nodes.

## Persistence

- All file persistence uses one installation profile: `%APPDATA%\schemata\<hash>` on Windows (the platform's data directory on Linux/macOS). `<hash>` is the 32-character lowercase hexadecimal MD5 of the canonical executable directory's UTF-8 path, used only as a directory identifier, not for security; Windows strips verbatim prefixes through `dunce::canonicalize` and lowercases the path before hashing. AppImage builds use the original AppImage location, not the temporary mount directory. The executable filename, application version and current working directory do not affect the profile.
- The root is configured through Tauri's `appDirectoriesOverride` before plugins or windows initialize. Config, data and local data resolve to that root; cache/log files stay below it. This also puts Windows WebView2's profile (frontend preferences, favorites, workspace and theme) and the window-state plugin's JSON under the installation profile. Connection definitions, query history and saved queries (`queries.json`) are JSON files in that root; connection files never contain passwords. History and saved queries are replaced atomically, and a damaged file is preserved beside it (`history-corrupt-*.json`, `queries-corrupt-*.json`) without preventing startup.
- Passwords stay in the OS credential store via `keyring`, with a service namespace containing the installation hash and entries keyed by connection id. If the store is unavailable, the password is not saved and the user is prompted on connect. A failed read while saving keeps password saving enabled and reports a warning. Connections with password saving disabled never read an old stored credential. Deleting a connection reports a credential-store failure after the connection itself is removed.
- A process-lifetime profile lock prevents concurrent instances from overwriting saved state. Window size/position uses Tauri window-state; panel sizes and tree state use persisted frontend stores within the WebView profile.
- Favorites (`schematadb.favorites`) and tree filters (`schematadb.treeFilter`, without the search text) persist in frontend storage. Favorites follow table renames and are dropped with the object.
- The Create table dialog's "Add `created_at` and `updated_at`" choice (`schematadb.designerPrefs`, off by default) persists and applies to later new tables until changed; it appends two nullable `DATETIME` columns, skipping names the design already has.
- Appearance: `schematadb.appearance` persists the light/dark choice in frontend storage. `main.tsx` applies it to the document root before React renders, so panes and portal-based dialogs share the same CSS tokens, and calls `set_window_theme` so Windows 11 paints the frameless window's DWM border in that theme's `--border` color (light at startup until the frontend calls it, before the window is shown). The SQL editor also switches its CodeMirror theme for syntax highlighting.
- No legacy imports, migrations, fallback directories or keyring fallback reads. Old ProjectCat, old release and old `.dev` directories are left untouched and ignored. Close the app before deleting its hashed profile: the next launch recreates clean defaults with no connections, query history, saved queries or restored Windows frontend preferences. Old keyring entries may remain in the OS vault, but no connection ids are restored to reference them.
- Isolation follows the installation directory, not the debug/release flag. Builds in different directories are isolated; builds launched from the same directory intentionally share one profile. Moving the app creates a new empty profile; updating the executable in the same directory retains the existing profile.

## MySQL vs MariaDB

- One driver serves both, detecting the flavor from the server version. Known divergences (JSON stored as LONGTEXT on MariaDB, UUID type, metadata column differences) are why the core scenario suite runs against both servers.

## Testing

- Core tests exercise the public `schematadb-core` API against real engines (SQLite files, MySQL and MariaDB in Docker), running the same scenarios on all three. Assert resulting database state, returned values or produced files; generated SQL is verified by executing it, not by string matching.
- The MySQL/MariaDB tests start reusable Docker containers `schematadb-test-mysql` (MySQL 8.4) and `schematadb-test-mariadb` (MariaDB 11.4); remove them with `docker rm -f` when done.
- Frontend: Vitest on pure state modules (grid pending changes, import column mapping, etc.). No automated UI end-to-end tests; UI is validated manually.

## Packaging

- All `make` targets need GNU Make and Node 24. `make build` also needs pnpm, Rust and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for the current OS; on Windows, Inno Setup 6.6+ (`ISCC.exe` in `PATH` or `C:\Program Files (x86)\Inno Setup 6`). Docker targets (`make linux`: Ubuntu 24.04 x86_64 AppImage; `make mac`: OSXCross ARM64 `.pkg`) need only Docker with Linux containers and BuildKit (x86_64 emulation on ARM64 hosts). Temporary files stay in `target/package-native/`; installers go to `package/<os>/`.
- Windows uses the root `setup.iss`, with `ProjectDir` resolved from the script location (`{#ProjectDir}` is an Inno preprocessor value, not a runtime constant). The installer reads its version from the Cargo-built executable and includes `LICENSE` and `THIRD_PARTY_LICENSES.txt`; the frontend, SQLite and application icon are embedded in the executable. `src-tauri/icons/icon.ico` contains multiple sizes converted from `designs/icons/app-icon-1024.png` and is also the setup icon. The installer supports English and Brazilian Portuguese, Start Menu and optional desktop shortcuts, and uninstall without deleting saved user data. If WebView2 is missing, setup downloads and installs Microsoft's Evergreen bootstrapper (needs internet). After a native build, `ISCC.exe setup.iss` recompiles only the installer, from any working directory when given the script's full path. The first Inno installation does not migrate an older NSIS installation; uninstall the NSIS version first (preserving user data).
- `make mac` uses the Apple SDK from [`rust-linux-darwin-builder`](https://github.com/joseluisq/rust-linux-darwin-builder), subject to Apple's terms. It produces an ad-hoc signed ARM64 `.pkg` installing into `/Applications` (macOS 11+, Apple Silicon, no universal binary); not a DMG because Tauri's DMG bundler needs macOS tools. Installers have no Developer ID signature or notarization, so Gatekeeper may block them; installation was not verified on a Mac.
- On Linux, builds need `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`; saved passwords need a Secret Service (GNOME Keyring or KWallet), otherwise the app asks for the password when connecting.

## Limitations

- English UI only.
- No SSH tunnels, TLS options beyond driver defaults, user/privilege management, routine designer, visual view designer, or schema/data compare and sync.
- Transactional grid edits and test-data replacement require a transactional table engine.
- The designer refuses changes it cannot reproduce without losing constraints or column attributes; generated columns are read-only.
- Disconnect a connection before changing its host, user, database, engine or SQLite file.

## Data-safety review (2026-11-12)

- Third review (three read-only audits of the data-edit, designer/DDL and bulk-transfer chains) focused on anything that can corrupt or silently lose data; every finding was fixed, each with a regression test where the path is testable. Rust workspace: 360 tests passed against SQLite, MySQL 8.4 and MariaDB 11.4; Clippy clean. Frontend: typecheck and 114 Vitest tests passed. UI smoke on the dev build: a multi-line cell opened and saved without typing staged nothing; an edit kept its line breaks in the database; Delete Table on a parent listed the cascading child table.

## Release-candidate validation (2026-11-11)

- Second full review (four read-only slice audits) found and fixed data-safety issues in editor transactions, by-value row matching, functional indexes, MySQL import retries, SQLite rebuilds, generation, CSV import mapping, operation cancellation, grid/designer input loss and the WebView2/second-instance startup paths; each fix has a regression test where the path is testable.
- Rust workspace: 237 tests passed, including real SQLite, MySQL 8.4 and MariaDB 11.4 scenarios; workspace Clippy passed with warnings denied. Frontend: production build/typecheck, 76 Vitest tests and third-party license verification passed; the production npm dependency audit found no known vulnerabilities.
- Real Windows UI smoke on the dev build: a second launch exited and kept one window; two runs in one query tab shared the same MySQL connection id and user variable; closing the tab removed that server connection; Ctrl+T behind a confirmation dialog opened no tab; a cell edit followed by Ctrl+D survived as a pending change (then discarded, nothing committed).
- The approved native-close path requires both `core:window:allow-close` and `core:window:allow-destroy`: Tauri's `onCloseRequested` implementation calls `destroy()` after an event is accepted.
- Installers under `package/` predate these fixes and must be rebuilt (`make`, `make linux`, `make mac`) before distribution.

## Pending

- There is no CI workflow. Linux/macOS builds, `.deb`/`.dmg` output, and installation/runtime smoke tests on Linux and macOS remain unverified, as does `tauri-plugin-single-instance` on Linux/macOS.
- SQLite rowid tables accept NULL in non-INTEGER primary keys; such rows are reported as vanished on edit (fail-safe) instead of being editable.
- Dependency audit on 2026-11-11: RustSec reports soundness warnings for Linux transitive dependencies `glib 0.18.5` (`RUSTSEC-2024-0429`, through Tauri/GTK) and `memmap2 0.5.10` (`RUSTSEC-2026-0186`, through decorum/linicon). Their patched major/minor versions are not drop-in replacements for the upstream dependency requirements. These warnings are not a clean Linux security approval. Five additional unmaintained-crate warnings remain; the audit reports no vulnerability errors.
