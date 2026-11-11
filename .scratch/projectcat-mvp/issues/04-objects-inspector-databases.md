# 04: Lista de objetos, inspector e bancos

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 23-28, 32-35, 50 (definição de view).

**O que entregar:** `table_info`, lista de objetos ordenável (Name, Rows, Modified, Comment), inspector com metadados, criar/excluir banco (charset/collation), attach/detach SQLite, breadcrumb e estado de conexão na status bar.

**Bloqueado por:** 03

**Status:** ready-for-agent

## Critérios de aceite
- [ ] Lista de objetos e inspector exibem metadados corretos nos 3 engines
- [ ] Criar/excluir banco atualiza a árvore; exclusão exige digitar o nome
- [ ] Attach/detach de arquivo SQLite funciona

## Evidências de implementação
