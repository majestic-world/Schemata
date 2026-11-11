# 08: Editor SQL

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 68-80.

**O que entregar:** Abas de consulta com CodeMirror (dialeto, autocomplete), divisão de script com DELIMITER, resultados por instrução com tempo e linhas afetadas, limite 1.000 + carregar mais, cancelamento, parada no erro, histórico por conexão, salvar/abrir .sql e exportar resultado.

**Bloqueado por:** 05

**Status:** implemented

## Critérios de aceite
- [x] Script com várias instruções gera um resultado por instrução
- [x] Cancelamento interrompe consulta longa nos 3 engines
- [x] Histórico e arquivos .sql funcionam

## Evidências de implementação

- Commit `365d171`
- `cargo test --test scripts`: 17 passed (divisão com aspas/comentários/escapes/DELIMITER/triggers, resultado por instrução com linhas afetadas e colunas de resultado vazio, parada no primeiro erro, load more, cancelamento nos 3 engines, colunas para autocomplete, histórico persistido)
- UI via CDP: script com 5 instruções → Summary com linhas retornadas/afetadas e erro do servidor na 4ª, abas Result 1/2; 1.000 linhas + 'Load 1000 more' → 2.000; Stop interrompe SLEEP(20) em 0,1 s; autocomplete sugere tabela; painel de histórico; marcador de alterações não salvas
- Não exercitado via CDP: diálogos nativos de abrir/salvar .sql (comandos de leitura/escrita são wrappers diretos de `tokio::fs`)
