# 06: Grid de dados: edição com change set

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 55-63.

**O que entregar:** Change set (update/insert/delete) acumulado no frontend, aplicado atomicamente no core com identificação da falha, detecção de somente leitura e rowid no SQLite.

**Bloqueado por:** 05

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Edições pendentes destacadas, commit em uma transação e descarte
- [ ] Falha mantém pendências e aponta a linha
- [ ] Sem identidade de linha → somente leitura com aviso; SQLite usa rowid
- [ ] Vitest cobre o estado de pendências (seam 2)

## Evidências de implementação
