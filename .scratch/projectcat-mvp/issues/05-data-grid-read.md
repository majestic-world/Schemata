# 05: Grid de dados: leitura paginada

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 29, 30, 51-54, 56 (exibição), 64, 65, 66, 67.

**O que entregar:** `fetch_page` com ordenação e filtros AND, modelo de valores sem perda de precisão, grid virtualizado, NULL distinto, binário em hex, cópia TSV, refresh, toggles Data/Structure/Info e contagem na status bar.

**Bloqueado por:** 04

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Grid pagina, ordena e filtra no servidor
- [ ] BIGINT/DECIMAL sem perda; NULL distinto de string vazia; binário como hex + tamanho
- [ ] Cópia de células em TSV

## Evidências de implementação
