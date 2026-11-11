# 04: Lista de objetos, inspector e bancos

**Fonte:** `.scratch/projectcat-mvp/spec.md`, histórias 23-28, 32-35, 50 (definição de view).

**O que entregar:** `table_info`, lista de objetos ordenável (Name, Rows, Modified, Comment), inspector com metadados, criar/excluir banco (charset/collation), attach/detach SQLite, breadcrumb e estado de conexão na status bar.

**Bloqueado por:** 03

**Status:** implemented

## Critérios de aceite
- [x] Lista de objetos e inspector exibem metadados corretos nos 3 engines
- [x] Criar/excluir banco atualiza a árvore; exclusão exige digitar o nome
- [x] Attach/detach de arquivo SQLite funciona

## Evidências de implementação

- Commit `ed5a517`
- `cargo test --test metadata`: 15 passed (table_info com colunas/índices/FKs/defaults/identidade de linha, CREATE reexecutável, criar/excluir banco com charset/collation, attach/detach SQLite) nos 3 engines
- UI via CDP: lista de objetos ordenável (tabelas antes de views), inspector com Rows/Created/Modified/Comment/Engine/Charset/Collation, breadcrumb na status bar; criar banco latin1 e excluir exigindo digitar o nome; attach/detach de arquivo SQLite
