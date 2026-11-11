Status: ready-for-agent

# ProjectCat MVP — cross-platform database client

## Problem Statement

Developers who manage MySQL, MariaDB and SQLite databases need a desktop client to browse schemas, edit data, run SQL and move data in and out. Navicat covers this but is heavyweight, paid and far broader than what day-to-day work needs. The user wants a focused, good-looking client — inspired by Navicat and visually based on `designs/04-glass-gradient.png` without the glass effect — that runs identically on Windows, Linux and macOS and offers only the essentials: manage connections, databases, tables and data, run queries, import, export, back up and restore.

## Solution

ProjectCat is a desktop application built with Tauri 2 (Rust backend) and a React + TypeScript frontend. Its window follows design 04 with solid surfaces: a top toolbar, a left sidebar with the connection tree, a central tabbed workspace (object list, table data, table designer, SQL query tabs), a right inspector panel and a bottom status bar. Users register MySQL, MariaDB and SQLite connections, browse databases, tables and views, create and drop databases and tables, design tables visually with a DDL preview, edit rows in a grid with explicit commit or discard, run SQL scripts with multiple results and cancellation, export tables or query results to CSV/JSON/SQL, import CSV files and SQL scripts, and back up or restore whole databases as `.sql` dumps. Passwords live in the operating system credential store. Installers are produced for all three platforms by CI.

## User Stories

### Connections

1. As a developer, I want to create a MySQL connection with host, port, user, password and optional default database, so that I can reach my MySQL server.
2. As a developer, I want to create a MariaDB connection with the same fields, so that I can reach my MariaDB server.
3. As a developer, I want to create a SQLite connection by picking an existing database file, so that I can open local databases.
4. As a developer, I want to create a SQLite connection that creates a new database file at a chosen path, so that I can start a new local database.
5. As a developer, I want to give each connection a display name and a color tag, so that I can tell environments apart in the tree.
6. As a developer, I want to test a connection before saving it, so that I know the settings are correct.
7. As a developer, I want a clear error message when a connection test fails, so that I can fix the host, credentials or file path.
8. As a developer, I want to edit an existing connection, so that I can update changed credentials or hosts.
9. As a developer, I want to duplicate a connection, so that I can quickly create a similar one for another environment.
10. As a developer, I want to delete a connection after confirming, so that I can remove obsolete entries.
11. As a security-conscious developer, I want passwords stored in the operating system credential store and never in plain-text config, so that my credentials are not exposed.
12. As a developer on a Linux machine without a credential service, I want to be prompted for the password when connecting, so that I can still use the app.
13. As a developer, I want to choose not to save a password, so that I am asked for it each time I connect.
14. As a developer, I want to open and close a connection explicitly, so that I control when sessions to my servers exist.
15. As a developer, I want my saved connections to persist across app restarts, so that I don't have to recreate them.
16. As a developer, I want a lost server connection to be detected and reconnected on the next action, so that a dropped network does not force me to restart the app.

### Navigation and inspector

17. As a developer, I want a sidebar tree of connection › database › Tables/Views, so that I can navigate my servers like in Navicat.
18. As a developer, I want tree nodes to load lazily when expanded, so that large servers open quickly.
19. As a developer, I want each Tables/Views group to show an item count, so that I can gauge a schema's size at a glance.
20. As a developer, I want to filter the tree with the top search field, so that I can find a table by name quickly.
21. As a developer, I want to refresh any tree node, so that I see changes made outside the app.
22. As a developer, I want right-click context menus on tree nodes with the relevant actions, so that common tasks are one click away.
23. As a developer, I want selecting Tables to show an object list with Name, Rows, Modified Date and Comment columns, so that I get an overview of the schema.
24. As a developer, I want to sort the object list by any column, so that I can find the largest or most recently changed tables.
25. As a developer, I want the right inspector to show table information (rows, created date, modified date, comment) for the selected object, so that I can inspect it without opening it.
26. As a developer, I want the inspector to show database, engine, charset and collation where the engine supports them, so that I understand the table's storage settings.
27. As a developer, I want the inspector to show a column count and a shortcut that opens the table, so that I can jump into the data.
28. As a developer, I want the status bar to show connection state and a breadcrumb of connection › database › object, so that I always know where I am.
29. As a developer, I want the status bar Data / Structure / Info toggles to switch the current table tab's view, so that I can move between data, design and metadata.
30. As a developer, I want the status bar to show the current row count, so that I know how much data I'm looking at.
31. As a developer, I want resizable sidebar and inspector panels, and to collapse the inspector, so that I can give the grid more room.

### Databases

32. As a developer, I want to create a MySQL/MariaDB database choosing charset and collation, so that it matches my application's needs.
33. As a developer, I want to drop a database after typing its name to confirm, so that I can't delete one by accident.
34. As a developer, I want to attach and detach additional SQLite database files on a SQLite connection, so that I can work with several files side by side.
35. As a developer, I want the tree to update right after creating or dropping a database, so that the UI reflects the server state.

### Tables and designer

36. As a developer, I want to create a table in a visual designer by adding columns with name, type, length/precision, nullability, default, auto-increment and comment, so that I don't have to write DDL by hand.
37. As a developer, I want type suggestions appropriate to the connection's engine, so that I only pick valid types.
38. As a developer, I want to define a primary key with one or more columns, so that rows are uniquely identified.
39. As a developer, I want to add, edit and remove indexes (unique or not) in the designer, so that I can tune queries.
40. As a developer, I want to add and remove basic foreign keys with referenced table, columns and on update/delete actions, so that I can model relationships.
41. As a developer, I want to set table options (engine, charset, collation, comment) on MySQL/MariaDB, so that tables are created as intended.
42. As a developer, I want to reorder, rename and drop columns of an existing table, so that I can evolve the schema.
43. As a developer, I want to see the exact DDL the designer will run before applying, so that I can review and copy it.
44. As a developer, I want a failed alteration to leave the table untouched and show the error, so that I never end up with a half-altered table.
45. As a SQLite user, I want column changes that SQLite's ALTER TABLE does not support to be applied safely by rebuilding the table while preserving data, indexes and triggers, so that I can alter SQLite tables like any other.
46. As a developer, I want to drop a table after confirming, so that I can remove unused tables.
47. As a developer, I want to empty a table (truncate) after confirming, so that I can reset its data while keeping the structure.
48. As a developer, I want to rename a table, so that I can fix naming.
49. As a developer, I want to copy a table's CREATE statement to the clipboard, so that I can reuse it elsewhere.
50. As a developer, I want to open a view's data and see its definition, so that I can inspect views even though I create them via SQL.

### Data grid

51. As a developer, I want to open a table's data in a paginated grid, so that I can browse large tables without loading everything.
52. As a developer, I want the grid to scroll smoothly with virtualization, so that wide or long pages stay responsive.
53. As a developer, I want to sort by clicking a column header, so that I can order the data server-side.
54. As a developer, I want a simple filter (column, operator, value) combined with AND, so that I can narrow down rows without writing SQL.
55. As a developer, I want to edit a cell inline, so that I can fix data quickly.
56. As a developer, I want to set a cell to NULL explicitly and see NULL rendered distinctly from an empty string, so that I never confuse the two.
57. As a developer, I want to insert a new row, so that I can add data.
58. As a developer, I want to delete selected rows, so that I can remove data.
59. As a developer, I want pending edits, inserts and deletes highlighted until I commit or discard them, so that I know what will change.
60. As a developer, I want commit to apply all pending changes in a single transaction, so that a failure applies nothing.
61. As a developer, I want a commit error to keep my pending changes and point to the failing row, so that I can fix and retry.
62. As a developer, I want tables without a primary or unique key to open read-only with an explanation, so that edits never hit the wrong rows.
63. As a SQLite user, I want tables without a primary key to remain editable via rowid when available, so that typical SQLite tables are editable.
64. As a developer, I want large integers and decimals to display and save without precision loss, so that financial and ID data stays correct.
65. As a developer, I want binary columns shown as a truncated hex preview with their size, so that the grid stays readable.
66. As a developer, I want to copy selected cells as tab-separated text, so that I can paste into a spreadsheet.
67. As a developer, I want to refresh the grid, so that I see changes made by others.

### SQL editor

68. As a developer, I want to open a new query tab bound to a connection and database, so that I can write SQL.
69. As a developer, I want SQL syntax highlighting in the engine's dialect, so that queries are easy to read.
70. As a developer, I want autocomplete of keywords, tables and columns of the current database, so that I write queries faster.
71. As a developer, I want to run the whole script or only the selected text, so that I can execute parts of a script.
72. As a developer, I want scripts containing multiple statements, comments, quoted semicolons and DELIMITER blocks to split correctly, so that procedures and triggers run as written.
73. As a developer, I want each statement's result shown in its own result tab, with affected-row counts for non-SELECT statements, so that I can review every outcome.
74. As a developer, I want execution time per statement, so that I can spot slow queries.
75. As a developer, I want results capped by default at 1,000 rows with a "load more" action, so that a huge SELECT doesn't freeze the app.
76. As a developer, I want to cancel a running query, so that I can stop a mistake or a slow query.
77. As a developer, I want a statement error to stop the script and show the engine's message and the failing statement, so that I can fix it.
78. As a developer, I want a per-connection query history, so that I can rerun previous queries.
79. As a developer, I want unsaved query tabs to be marked and to save/open queries as `.sql` files, so that I don't lose work.
80. As a developer, I want to export a query result, so that I can share ad-hoc data.

### Export and import

81. As a developer, I want to export a table to CSV with delimiter, quote, header and encoding options, so that I can open it in other tools.
82. As a developer, I want to export a table to JSON (array of objects), so that I can feed it to applications.
83. As a developer, I want to export a table to SQL INSERT statements, optionally with its CREATE TABLE, so that I can recreate it elsewhere.
84. As a developer, I want export to stream to disk with a progress indicator and a cancel button, so that large tables export without exhausting memory.
85. As a developer, I want to import a CSV into an existing table with a preview and a source-to-target column mapping, so that differently shaped files can be loaded.
86. As a developer, I want the import to run in a transaction and report inserted and failed rows, so that I know the outcome and can roll back on error.
87. As a developer, I want to run a `.sql` script file against a database with progress and a stop-on-error option, so that I can load dumps from other tools.

### Backup and restore

88. As a developer, I want to back up a whole database to a `.sql` file choosing structure, data or both, so that I can keep a copy.
89. As a developer, I want the dump to include tables, views, indexes, foreign keys and (on MySQL/MariaDB) triggers in a dependency-safe order, so that it restores cleanly.
90. As a developer, I want to restore a `.sql` dump into a selected (possibly new) database, so that I can recover or clone data.
91. As a developer, I want backup and restore to show progress and be cancelable, so that long operations stay under control.

### Application and platform

92. As a Windows, Linux or macOS user, I want a native installer for my platform, so that I can install ProjectCat normally.
93. As a user, I want the window to use my platform's native title bar and controls, so that it feels at home on each OS.
94. As a user, I want the interface to follow design 04 with solid surfaces, a teal accent and no translucency, so that it looks clean and consistent across platforms.
95. As a user, I want long operations to never freeze the interface, so that I can keep working while they run.
96. As a user, I want errors shown as non-blocking notifications or inline panels with the engine's message, so that a failure never crashes the session.
97. As a keyboard user, I want shortcuts for new query (Ctrl/Cmd+T), run (Ctrl/Cmd+Enter), save (Ctrl/Cmd+S), commit grid changes and search (Ctrl/Cmd+K), so that I work faster.
98. As a user, I want window size, panel sizes and open connections' tree state remembered between sessions, so that the app reopens as I left it.

## Implementation Decisions

- **Stack:** Tauri 2 desktop shell; Rust backend; React + TypeScript frontend built with Vite and pnpm. Chosen over Qt (higher styling cost, licensing), Wails (less mature drivers/packaging) and pure-Rust/Go GUI toolkits (no mature data grid or code editor). Record this as the first ADR.
- **Modules:**
  - `projectcat-core` (Rust library, no Tauri dependency): drivers, value model, metadata, DDL generation, row-change application, script splitting, query execution, export, import, dump/restore, connection store and credential access. This is the testable heart of the app.
  - Tauri application: thin command layer that maps frontend calls to core services, converts errors to a serializable error type and forwards progress events. No business logic.
  - Frontend: layout shell, connection dialog, tree, object list, inspector, data grid, table designer, SQL editor, export/import/backup wizards, status bar.
- **Driver abstraction:** a `Driver` trait implemented by `MySqlDriver` (sqlx, serving both MySQL and MariaDB via the MySQL protocol, with flavor detection from the server version for dialect differences) and `SqliteDriver` (rusqlite with bundled SQLite, for interrupt support and full PRAGMA access). Capabilities: test, list databases, list objects, table info (columns, indexes, foreign keys, engine, charset, collation, row estimate, dates, comment, CREATE statement), fetch page (offset, limit, sort, filter), execute script returning per-statement results, cancel, apply row changes, create/alter DDL from a table design, stream rows.
- **Metadata sources:** `information_schema` plus `SHOW CREATE TABLE` on MySQL/MariaDB; `sqlite_schema` plus `pragma table_info / index_list / index_info / foreign_key_list` on SQLite. Row counts in the object list are estimates on MySQL/MariaDB (from `information_schema`) and exact counts on SQLite.
- **Value model:** cells cross the Rust↔JS boundary as a tagged union so precision and NULL are never lost. Shape:
  - `Null`, `Int(i64 within ±2^53)`, `BigInt(string)`, `Float(f64)`, `Decimal(string)`, `Text(string)`, `Bytes { preview_hex, len }`, `DateTime(string)`, `Json(string)`.
  - Writes accept the same union; values are always bound as prepared-statement parameters, never interpolated.
- **Row editing:** the frontend accumulates a change set (updates keyed by row identity, inserts, deletes). Row identity is the primary key, else a unique non-null key, else SQLite `rowid`; with none, the grid is read-only. The core applies the whole change set in one transaction and reports the first failing change.
- **Table design contract:** a `TableDesign` value (name, columns, primary key, indexes, foreign keys, options) is the single input for both create and alter. The core diffs old vs new designs and produces the DDL list shown in the preview; the same list is what gets executed. MySQL/MariaDB alterations run as `ALTER TABLE` statements. SQLite alterations unsupported by `ALTER TABLE` use the official rebuild procedure (foreign keys off, new table, copy data, drop, rename, recreate indexes and triggers, foreign-key check) inside one transaction.
- **Script execution:** a dialect-aware splitter handles quotes, backticks, comments and the `DELIMITER` directive. Each statement returns either a result set (columns + first page, default 1,000 rows, with continuation) or an affected-rows count, plus duration. Execution stops at the first error.
- **Cancellation:** MySQL/MariaDB cancel by issuing `KILL QUERY <connection id>` on an auxiliary connection; SQLite cancels via the interrupt handle. Export, import, dump and restore check a cancellation token between batches.
- **Long operations:** export, import, dump and restore stream in batches on the Rust side directly to/from files and emit progress events (processed rows/statements, total when known). The UI never holds full tables in memory.
- **Dump format:** plain `.sql` compatible with the engine's own CLI tools — on MySQL/MariaDB, CREATE statements with foreign-key checks disabled during load, batched multi-row INSERTs, views after tables, triggers last; on SQLite, schema SQL from `sqlite_schema` plus INSERTs wrapped in a transaction.
- **Persistence:** connection definitions in a JSON file in the platform app-config directory (no secrets). Passwords via the `keyring` crate (Windows Credential Manager, macOS Keychain, Linux Secret Service), keyed by connection id; if the credential store is unavailable, the password is not saved and the user is prompted on connect. Query history and UI state (window and panel sizes, expanded nodes) are stored in the app-data directory.
- **Errors:** a single serializable `AppError { kind, message, sql_state? }` crosses the command boundary; kinds distinguish connection, authentication, SQL, cancelled, I/O and validation errors.
- **Frontend libraries:** TanStack Table + TanStack Virtual (grid), CodeMirror 6 with `@codemirror/lang-sql` MySQL/SQLite dialects (editor), Radix UI primitives (menus, dialogs, tabs), lucide-react (icons), TanStack Query (server-state caching of metadata) and Zustand (UI and pending-change state).
- **Visual system:** design 04 without glass: solid light surfaces, white panels, subtle borders, teal accent, 10–12 px radii, system font stack, CSS custom-property tokens. Toolbar keeps Connection, New Query, Table, View, Backup, Import, Export; User, Function, Automation, Model and BI from the mockup are dropped. Native window decorations per OS.
- **Interface language:** English UI, matching the design.
- **Distribution:** GitHub Actions matrix (Windows, Ubuntu, macOS) using the Tauri action to build `.msi`/`.exe`, `.deb`/`.AppImage` and `.dmg`. Linux requires WebKitGTK 4.1.

## Testing Decisions

- **What makes a good test:** exercise externally observable behavior through public interfaces against real engines — given a database state and a service call, assert the resulting database state, returned values or produced file. No tests of internal helpers, call forwarding, mocks echoing inputs, or generated SQL text unless the text is the contract (the DDL preview is verified by executing it, not by string matching).
- **Seam 1 (primary) — `projectcat-core` public API**, integration tests against real engines: SQLite on temporary files, MySQL 8 and MariaDB 11 via testcontainers (Docker). The same scenario suite runs against all three engines. Coverage:
  - metadata listing (databases, objects, table info, CREATE statement);
  - create/alter/drop via `TableDesign`, including SQLite rebuild with data, indexes, triggers and foreign keys preserved, and atomic failure leaving the table unchanged;
  - value round-trip at the boundaries (max BIGINT, unsigned BIGINT, high-precision DECIMAL, binary, dates, JSON, NULL vs empty string);
  - row change sets applied atomically, with failure reporting; read-only detection without row identity; SQLite rowid editing;
  - script splitting with quotes, comments and DELIMITER blocks; per-statement results and error stop;
  - cancellation of a long-running query on each engine;
  - export → import round-trips (CSV, JSON, SQL) preserving counts and values; CSV column mapping;
  - dump → restore into an empty database reproducing schema and data;
  - connection store persistence without secrets.
- **Seam 2 (secondary) — frontend pure state modules** via Vitest: grid pending-change state (edit/insert/delete/discard/commit-failure retention) and import column mapping.
- **UI:** no automated E2E. Manual validation per milestone on Windows via omp computer use (per `AGENTS.md`), plus CI builds on all three platforms; Linux/macOS artifacts are smoke-checked.
- **CI:** core tests on Linux with MySQL/MariaDB containers; Vitest; builds on the three OSes.
- **Prior art:** none — the repository has no code yet; these suites establish the conventions.

## Out of Scope

- User and privilege management, functions/procedures designer, automation/scheduling, data modeling, BI/charts, cloud sync.
- Engines other than MySQL, MariaDB and SQLite.
- SSH tunnels and advanced SSL/TLS options (basic TLS defaults of the driver only).
- Visual view designer (views are created through the SQL editor).
- Data and structure synchronization/compare between databases.
- Dark theme (tokens allow adding it later).
- Code signing/notarization of installers.
- Localization beyond English.

## Further Notes

- Docker must be running locally for the MySQL/MariaDB core tests; the daemon (Linux containers) is running on the development machine.
- macOS builds require a macOS CI runner; the development machine is Windows.
- MariaDB and MySQL share one driver; known divergences (JSON stored as LONGTEXT on MariaDB, UUID type, metadata column differences) must be covered by running the scenario suite on both.
- Suggested delivery order: scaffold → design shell → core drivers → connections and tree → object list and inspector → data grid / table designer / SQL editor (parallelizable) → export and import → backup and restore → packaging and CI.
- Next step: run `/implement-plan-spec` on this spec.
