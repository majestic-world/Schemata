# 03: Driver MySQL/MariaDB e suíte multi-engine

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 1, 2, 6, 7, 16.

**O que entregar:** `MySqlDriver` (sqlx) com detecção de flavor, mesma suíte de cenários rodando em SQLite, MySQL 8 e MariaDB 11 via testcontainers; reconexão após perda de conexão.

**Bloqueado por:** 02

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Conectar, testar e navegar MySQL e MariaDB pela UI
- [ ] Suíte de cenários roda nos 3 engines
- [ ] Conexão perdida é restabelecida na próxima ação

## Evidências de implementação
