# 09: Exportar e importar

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 80-87, 95.

**O que entregar:** Exportação em streaming de tabela/resultado para CSV/JSON/SQL com opções, progresso e cancelamento; importação CSV com prévia e mapeamento em transação; execução de script .sql com progresso e stop-on-error.

**Bloqueado por:** 06, 08

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Round-trip exportar → importar preserva contagens e valores
- [ ] Mapeamento de colunas no CSV
- [ ] Progresso e cancelamento sem travar a UI

## Evidências de implementação
