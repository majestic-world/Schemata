# 01: Scaffold do app e casca visual

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 92-94 (parcial), 31.

**O que entregar:** Workspace Cargo (app Tauri 2 + crate `projectcat-core`), frontend Vite/React/TS com pnpm, ADR da stack, tokens visuais do design 04 sem glass e layout de 3 colunas (toolbar, sidebar, abas, inspector, status bar) com painéis redimensionáveis e inspector recolhível.

**Bloqueado por:** Nenhum

**Status:** done

## Critérios de aceite
- [x] `pnpm tauri dev` abre a janela com decoração nativa
- [x] Layout segue o design 04 com superfícies sólidas e acento teal
- [x] Sidebar e inspector redimensionáveis; inspector recolhível
- [x] ADR 0001 registra a stack

## Evidências de implementação

- Commit `5e869d8`
- `pnpm typecheck` ok; `cargo build -p projectcat` ok
- UI via CDP (WebView2 remote debugging): janela abre, splitter redimensiona sidebar 280→360 px, toggle recolhe o inspector
