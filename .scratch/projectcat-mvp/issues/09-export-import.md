# 09: Exportar e importar

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 80-87, 95.

**O que entregar:** Exportação em streaming de tabela/resultado para CSV/JSON/SQL com opções, progresso e cancelamento; importação CSV com prévia e mapeamento em transação; execução de script .sql com progresso e stop-on-error.

**Bloqueado por:** 06, 08

**Status:** implemented

## Critérios de aceite
- [x] Round-trip exportar → importar preserva contagens e valores
- [x] Mapeamento de colunas no CSV
- [x] Progresso e cancelamento sem travar a UI

## Evidências de implementação

- Commit `ddb42d1`
- `cargo test --test transfer`: 16 passed (round-trip CSV com aspas/quebras/acentos, JSON com tipos, SQL com CREATE recriando a tabela, mapeamento de colunas, rollback no primeiro erro com linha reportada e modo pular, cancelamento, delimitador e Windows-1252) nos 3 engines
- `pnpm test`: mapeamento automático/único de colunas do CSV (seam 2)
- UI via CDP: importar CSV com ';' mapeando colunas fora de ordem e ignorando uma, progresso e relatório; executar arquivo .sql sem parar no erro → '3 statements executed, 1 failed' com a mensagem
- Não exercitado via CDP: diálogo nativo de salvar na exportação (lógica coberta pelos testes do core)
