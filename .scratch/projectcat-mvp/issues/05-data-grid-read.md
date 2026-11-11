# 05: Grid de dados: leitura paginada

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 29, 30, 51-54, 56 (exibição), 64, 65, 66, 67.

**O que entregar:** `fetch_page` com ordenação e filtros AND, modelo de valores sem perda de precisão, grid virtualizado, NULL distinto, binário em hex, cópia TSV, refresh, toggles Data/Structure/Info e contagem na status bar.

**Bloqueado por:** 04

**Status:** done

## Critérios de aceite
- [x] Grid pagina, ordena e filtra no servidor
- [x] BIGINT/DECIMAL sem perda; NULL distinto de string vazia; binário como hex + tamanho
- [x] Cópia de células em TSV

## Evidências de implementação

- Commit `6e2c070`
- `cargo test --test data`: 12 passed (paginação/ordenação no servidor, filtros AND com curingas LIKE literais, BIGINT/UNSIGNED/DECIMAL/binário/datas/TIME negativo/BIT/JSON/NULL vs vazio, rowid oculto no SQLite) nos 3 engines
- UI via CDP: grid virtualizado com tabela de 504 linhas, 100/página, última página 501–504, ordenação desc por coluna, filtro 'contains' (16 linhas), cópia TSV de seleção 2×2, toggles Data/Structure/Info e contagem na status bar; BIGINT UNSIGNED máximo exato, NULL distinto de vazio, BLOB em hex com tamanho
