# 0001: Tauri 2 + Rust + React/TypeScript stack

Status: accepted

## Context

SchemataDb is a desktop database client for Windows, Linux and macOS supporting MySQL, MariaDB and SQLite. The UI follows a custom visual design (data grid, SQL editor, tree, inspector) that native widget toolkits would make costly to reproduce. The development machine has Rust, Go and a C++ toolchain available.

## Decision

- Desktop shell: Tauri 2. Backend in Rust; frontend in React + TypeScript built with Vite and pnpm.
- Business logic lives in the `schematadb-core` crate, which has no Tauri dependency. The Tauri app is a thin command layer.
- Drivers: sqlx for MySQL/MariaDB (one driver, flavor detected from the server version); rusqlite with bundled SQLite for SQLite (interrupt support, full PRAGMA access).
- Frontend: TanStack Table/Virtual (grid), CodeMirror 6 (editor), Radix UI primitives, lucide-react icons, TanStack Query and Zustand for state.

## Alternatives

- Qt 6 (C++): best native fidelity, but styling to the design is expensive and licensing (LGPL/commercial) adds constraints.
- Wails (Go): same web UI approach with less mature drivers and packaging.
- Slint, egui, iced, Fyne: no mature data grid or code editor.

## Consequences

- Rendering depends on each OS webview (WebView2, WebKitGTK 4.1, WKWebView); avoid bleeding-edge CSS and smoke-test CI artifacts.
- Linux builds require WebKitGTK 4.1; credential storage requires a Secret Service, with a prompt-on-connect fallback.
