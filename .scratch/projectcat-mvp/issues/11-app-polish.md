# 11: Atalhos, estado persistido e erros

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 96-98.

**O que entregar:** Atalhos (Ctrl/Cmd+T, Enter, S, K, commit), persistência de tamanho de janela/painéis/nós expandidos, notificações de erro não bloqueantes.

**Bloqueado por:** 06, 07, 08

**Status:** implemented

## Critérios de aceite
- [x] Atalhos funcionam
- [x] App reabre como foi deixado
- [x] Erros aparecem sem derrubar a sessão

## Evidências de implementação

- Commit `8bf8475`
- `pnpm typecheck` ok
- UI via CDP: Ctrl+K foca a busca; Ctrl+T abre consulta no banco selecionado; Ctrl+S no grid faz commit (gravado no MySQL); Ctrl+Enter/Ctrl+S do editor via keymap (ticket 08)
- Reinício do app: conexão MySQL reaberta automaticamente com a árvore expandida e largura da sidebar preservada; fechamento normal grava `.window-state.json` (tamanho/posição da janela)
- Erros: toasts não bloqueantes (já existentes), `unhandledrejection` vira toast e ErrorBoundary isola falhas de renderização da área de trabalho
