# 03: Driver MySQL/MariaDB e suíte multi-engine

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 1, 2, 6, 7, 16.

**O que entregar:** `MySqlDriver` (sqlx) com detecção de flavor, mesma suíte de cenários rodando em SQLite, MySQL 8 e MariaDB 11 via testcontainers; reconexão após perda de conexão.

**Bloqueado por:** 02

**Status:** implemented

## Critérios de aceite
- [x] Conectar, testar e navegar MySQL e MariaDB pela UI
- [x] Suíte de cenários roda nos 3 engines
- [x] Conexão perdida é restabelecida na próxima ação

## Evidências de implementação

- Commit `90a9ae7`
- `cargo test --test metadata --test reconnect`: 8 passed (SQLite, MySQL 8.4, MariaDB 11.4 via testcontainers; reconexão após KILL de todas as conexões da sessão)
- UI via CDP: conexão MySQL testada (MySQL 8.4.13) e salva com senha no keyring; MariaDB sem salvar senha → teste mostra 'Access denied…'; abrir pede senha e expande a árvore
