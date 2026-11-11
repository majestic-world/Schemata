# Execução: ProjectCat MVP

- Fonte: `.scratch/projectcat-mvp/spec.md`
- Branch: `main`
- base_sha: `22f838e1dffa10af4871fe393937f2aeb7c808e9`
- Alterações preexistentes: nenhuma (árvore limpa)
- Seams de teste confirmados: (1) API pública do `projectcat-core` contra SQLite/MySQL 8/MariaDB 11; (2) módulos de estado puros do frontend via Vitest; UI validada manualmente via computer use.

## Tickets

| # | Ticket | Bloqueado por | Status | Commit |
|---|---|---|---|---|
| 01 | Scaffold do app e casca visual | Nenhum | done | `5e869d8` |
| 02 | Conexões SQLite, store, credenciais e árvore | 01 | done | `7f1354c` |
| 03 | Driver MySQL/MariaDB e suíte multi-engine | 02 | done | `90a9ae7` |
| 04 | Lista de objetos, inspector e bancos | 03 | done | `ed5a517` |
| 05 | Grid de dados: leitura paginada | 04 | done | `6e2c070` |
| 06 | Grid de dados: edição com change set | 05 | done | `79dce02` |
| 07 | Designer de tabela e operações de tabela | 05 | done | `d7c9872` |
| 08 | Editor SQL | 05 | done | `365d171` |
| 09 | Exportar e importar | 06, 08 | done | `ddb42d1` |
| 10 | Backup e restore | 09 | done | `3e10471` |
| 11 | Atalhos, estado persistido e erros | 06, 07, 08 | done | `8bf8475` |
| 12 | CI e empacotamento | 10, 11 | implemented | `9a18695` |

## Log

- Tickets 01–12 implementados com commits incrementais (ver tabela).
- Gates integrados: `cargo test -p projectcat-core` (107 testes em SQLite, MySQL 8.4 e MariaDB 11.4), `cargo clippy --workspace --all-targets` sem avisos, `pnpm typecheck`, `pnpm test` (8 testes Vitest), `pnpm tauri build` (MSI e NSIS), `actionlint`.
- Revisão rodada 1 (`code-review`, base `22f838e`):
  - Standards: imports dentro de funções, histórico calculado na camada Tauri, contagens sem flexão, seletor de arquivo SQL duplicado, exportação/backup com falha apagando arquivo existente, erro de histórico mascarando resultado de SQL → corrigidos em `6f6981b`.
  - Spec: restauração lendo arquivo inteiro na memória, edições pendentes perdidas ao trocar de aba/visão, restore/backup SQLite ignorando o banco anexado, cancelamento SQLite deixando transação aberta → corrigidos em `c5db698` e `da0aa21`; adicionada opção de aspas no export CSV (história 81).
- Revisão rodada 2: temporário de exportação previsível, cancelamento não reportado ao fechar o canal, import dentro de closure, edições pendentes reatribuíveis após refetch, restore SQLite em banco anexado → corrigidos em `6f972e5`. Rodada 3 (Standards e Spec): nenhum achado restante.
- Gates finais: 109 testes do core, clippy limpo, typecheck, 8 testes Vitest.
- Pendência: executar o workflow de CI no GitHub (jobs Linux/macOS e instaladores .deb/.AppImage/.dmg) após push; ticket 12 permanece `implemented` até lá.
- Próximo passo: push e acompanhamento do CI (fora da autorização desta execução).
