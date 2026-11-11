# 07: Designer de tabela e operações de tabela

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 36-49.

**O que entregar:** `TableDesign` para criar/alterar com prévia de DDL, tipos por engine, PK, índices, FKs, opções MySQL, reordenar/renomear/remover colunas, rebuild seguro no SQLite, excluir, truncar, renomear e copiar CREATE.

**Bloqueado por:** 05

**Status:** implemented

## Critérios de aceite
- [x] Criar e alterar tabelas nos 3 engines usando o DDL exibido
- [x] Falha não deixa alteração parcial
- [x] Rebuild SQLite preserva dados, índices, triggers e FKs

## Evidências de implementação

- Commit `d7c9872`
- `cargo test --test design`: 15 passed (criar com PK/auto-inc/default/índice único/FK/comentário, design relido não gera DDL, alterar renomeando/reordenando/mudando tipo/adicionando e removendo colunas e renomeando tabela com dados preservados, falha atômica nos 3 engines, rebuild SQLite preservando índice/trigger/FK/view, drop/truncate/rename) — corrigiu o divisor de scripts para `END;` de triggers
- UI via CDP: criar tabela MySQL com coluna NOT NULL, DEFAULT CURRENT_TIMESTAMP e índice único com prévia; alterar renomeando coluna (índice acompanha), adicionando e reordenando em um único ALTER; renomear e excluir tabela; alterar tipo no SQLite gera e aplica o rebuild recriando a view dependente
