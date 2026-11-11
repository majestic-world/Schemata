# 08: Editor SQL

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 68-80.

**O que entregar:** Abas de consulta com CodeMirror (dialeto, autocomplete), divisão de script com DELIMITER, resultados por instrução com tempo e linhas afetadas, limite 1.000 + carregar mais, cancelamento, parada no erro, histórico por conexão, salvar/abrir .sql e exportar resultado.

**Bloqueado por:** 05

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Script com várias instruções gera um resultado por instrução
- [ ] Cancelamento interrompe consulta longa nos 3 engines
- [ ] Histórico e arquivos .sql funcionam

## Evidências de implementação
