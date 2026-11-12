# Schemata

Desktop database client for MySQL, MariaDB and SQLite, inspired by Navicat. Built with Tauri 2, Rust and React.

## Development

Needs Rust (stable), Node 24 with pnpm, and Docker for the MySQL/MariaDB tests.

```bash
pnpm install
pnpm tauri dev                  # run the app
pnpm typecheck && pnpm test     # frontend checks
cargo test -p schematadb-core   # core tests (SQLite, MySQL, MariaDB)
make build                      # installer for the current OS, in package/
```

Design notes, packaging details and limitations: [docs/architecture.md](docs/architecture.md).

## License

MIT, see [LICENSE](LICENSE). Third-party licenses: [THIRD_PARTY_LICENSES.txt](THIRD_PARTY_LICENSES.txt) (run `pnpm notices` after changing dependencies).
