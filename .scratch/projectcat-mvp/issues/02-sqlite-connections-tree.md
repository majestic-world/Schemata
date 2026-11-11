# 02: Conexões SQLite, store, credenciais e árvore

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 1-15, 17-22.

**O que entregar:** Contrato `Driver`/`CellValue`/`AppError` no core, `SqliteDriver`, store de conexões em JSON sem segredos + keyring, comandos Tauri e UI de conexão (criar/testar/editar/duplicar/excluir, cor) e árvore lazy conexão › banco › Tables/Views com contagens, filtro e refresh.

**Bloqueado por:** 01

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Conexão SQLite (arquivo existente ou novo) criada, testada e persistida entre reinícios
- [ ] Senha nunca aparece no JSON; sem keyring o app pede senha ao conectar
- [ ] Árvore expande lazy, mostra contagens, filtra e atualiza
- [ ] Testes do core (seam 1) cobrem store e listagem no SQLite

## Evidências de implementação
