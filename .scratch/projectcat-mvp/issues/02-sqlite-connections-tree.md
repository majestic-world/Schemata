# 02: Conexões SQLite, store, credenciais e árvore

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 1-15, 17-22.

**O que entregar:** Contrato `Driver`/`CellValue`/`AppError` no core, `SqliteDriver`, store de conexões em JSON sem segredos + keyring, comandos Tauri e UI de conexão (criar/testar/editar/duplicar/excluir, cor) e árvore lazy conexão › banco › Tables/Views com contagens, filtro e refresh.

**Bloqueado por:** 01

**Status:** implemented

## Critérios de aceite
- [x] Conexão SQLite (arquivo existente ou novo) criada, testada e persistida entre reinícios
- [x] Senha nunca aparece no JSON; sem keyring o app pede senha ao conectar
- [x] Árvore expande lazy, mostra contagens, filtra e atualiza
- [x] Testes do core (seam 1) cobrem store e listagem no SQLite

## Evidências de implementação

- Commit `7f1354c`
- `cargo test --test connections`: 5 passed (store sem segredos, keyring indisponível, criação de arquivo SQLite, listagem com contagens exatas)
- UI via CDP: criar conexão SQLite com teste (SQLite 3.54.1), salvar com cor, expandir árvore com contagens, filtro pela busca, duplicar e excluir pelo menu de contexto; `connections.json` sem senha
