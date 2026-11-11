# 12: CI e empacotamento

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 92.

**O que entregar:** GitHub Actions: testes do core no Linux com MySQL/MariaDB, Vitest, build Tauri em Windows, Ubuntu e macOS gerando instaladores.

**Bloqueado por:** 10, 11

**Status:** implemented

## Critérios de aceite
- [ ] Workflow válido gera .msi/.exe, .deb/.AppImage e .dmg (workflow validado com actionlint; execução real depende de push)
- [x] Build local do Windows gera instalador

## Evidências de implementação

- Commit `9a18695`
- `pnpm tauri build` local: `ProjectCat_0.1.0_x64_en-US.msi` (6,95 MiB) e `ProjectCat_0.1.0_x64-setup.exe` (4,89 MiB); executável release abre a janela (screenshot)
- `actionlint` no workflow: sem erros
- Pendente de execução real: jobs do GitHub Actions (Linux/macOS e geração de .deb/.AppImage/.dmg) só rodam após push
