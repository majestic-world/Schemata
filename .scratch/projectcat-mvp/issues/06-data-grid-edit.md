# 06: Grid de dados: edição com change set

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 55-63.

**O que entregar:** Change set (update/insert/delete) acumulado no frontend, aplicado atomicamente no core com identificação da falha, detecção de somente leitura e rowid no SQLite.

**Bloqueado por:** 05

**Status:** implemented

## Critérios de aceite
- [x] Edições pendentes destacadas, commit em uma transação e descarte
- [x] Falha mantém pendências e aponta a linha
- [x] Sem identidade de linha → somente leitura com aviso; SQLite usa rowid
- [x] Vitest cobre o estado de pendências (seam 2)

## Evidências de implementação

- Commit `79dce02`
- `cargo test --test edit`: 15 passed (change set atômico, rollback com índice da falha, linha sumida, update sem mudança no MySQL, somente leitura sem identidade, rowid no SQLite, round-trip de BIGINT mín./UNSIGNED máx./DECIMAL/JSON/datetime/NULL vs '') — corrigiu overflow em `i64::MIN`
- `pnpm test` (Vitest, seam 2): 5 testes do estado de pendências
- UI via CDP: editar, inserir e excluir com destaque, commit aplicado no MySQL; PK duplicada → toast com mensagem do servidor, linha marcada, pendências mantidas e banco intacto; tabela sem chave mostra badge somente leitura; SQLite sem PK editado via rowid
