# 07: Designer de tabela e operações de tabela

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 36-49.

**O que entregar:** `TableDesign` para criar/alterar com prévia de DDL, tipos por engine, PK, índices, FKs, opções MySQL, reordenar/renomear/remover colunas, rebuild seguro no SQLite, excluir, truncar, renomear e copiar CREATE.

**Bloqueado por:** 05

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Criar e alterar tabelas nos 3 engines usando o DDL exibido
- [ ] Falha não deixa alteração parcial
- [ ] Rebuild SQLite preserva dados, índices, triggers e FKs

## Evidências de implementação
