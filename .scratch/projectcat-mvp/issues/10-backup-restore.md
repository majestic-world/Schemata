# 10: Backup e restore

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 88-91.

**O que entregar:** Dump .sql de banco (estrutura/dados/ambos) em ordem segura e restauração em banco selecionado ou novo, com progresso e cancelamento.

**Bloqueado por:** 09

**Status:** implemented

## Critérios de aceite
- [x] Dump restaurado em banco vazio reproduz estrutura e dados nos 3 engines

## Evidências de implementação

- Commit `3e10471`
- `cargo test --test backup`: 6 passed (dump completo restaurado em banco vazio reproduz objetos, colunas, índices, FKs, linhas, view sobre view, trigger e auto-incremento; dump só de estrutura) nos 3 engines
- UI via CDP: 'Restore Backup…' de um arquivo gerado pelo `mysqldump` em banco novo (aw_copy) → 64 instruções, tabelas e 3 linhas restauradas; menu do banco com Backup/Restore
- Não exercitado via CDP: diálogo nativo de salvar do Backup (lógica coberta pelos testes do core)
