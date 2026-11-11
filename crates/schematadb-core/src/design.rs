//! Table designs and the DDL that creates or alters a table to match one.

use crate::{IndexKind, IndexMethod, TableInfo, model::ObjectKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDesign {
    /// Name of the column in the original table; `None` for new columns. Drives rename detection.
    pub original_name: Option<String>,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// SQL expression, e.g. `'text'`, `0`, `CURRENT_TIMESTAMP`.
    pub default: Option<String>,
    pub auto_increment: bool,
    pub primary_key: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDesign {
    pub name: String,
    pub columns: Vec<String>,
    pub kind: IndexKind,
    /// `USING` clause on MySQL/MariaDB `Normal`/`Unique` indexes; `None` keeps the engine default.
    pub method: Option<IndexMethod>,
    /// MySQL/MariaDB only.
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyDesign {
    pub name: String,
    pub columns: Vec<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_update: String,
    pub on_delete: String,
}

/// MySQL/MariaDB table options; ignored on SQLite.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOptions {
    pub engine: Option<String>,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub comment: Option<String>,
}

/// Single input for creating and altering tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDesign {
    pub name: String,
    pub columns: Vec<ColumnDesign>,
    pub indexes: Vec<IndexDesign>,
    pub foreign_keys: Vec<ForeignKeyDesign>,
    pub options: TableOptions,
}

impl TableDesign {
    pub fn from_info(info: &TableInfo) -> TableDesign {
        debug_assert_eq!(info.kind, ObjectKind::Table);
        TableDesign {
            name: info.name.clone(),
            columns: info
                .columns
                .iter()
                .map(|c| ColumnDesign {
                    original_name: Some(c.name.clone()),
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    nullable: c.nullable,
                    default: c.default.clone(),
                    auto_increment: c.auto_increment,
                    primary_key: c.primary_key,
                    comment: c.comment.clone(),
                })
                .collect(),
            indexes: info
                .indexes
                .iter()
                .map(|i| IndexDesign {
                    name: i.name.clone(),
                    columns: i.columns.clone(),
                    kind: i.kind,
                    method: i.method,
                    comment: i.comment.clone(),
                })
                .collect(),
            foreign_keys: info
                .foreign_keys
                .iter()
                .map(|f| ForeignKeyDesign {
                    name: f.name.clone(),
                    columns: f.columns.clone(),
                    ref_table: f.ref_table.clone(),
                    ref_columns: f.ref_columns.clone(),
                    on_update: f.on_update.clone(),
                    on_delete: f.on_delete.clone(),
                })
                .collect(),
            options: TableOptions {
                engine: info.engine.clone(),
                charset: info.charset.clone(),
                collation: info.collation.clone(),
                comment: info.comment.clone(),
            },
        }
    }

    fn primary_key(&self) -> Vec<String> {
        self.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect()
    }
}

/// Maps original column names to their names in the new design.
fn rename_map(design: &TableDesign) -> HashMap<String, String> {
    design.columns.iter().filter_map(|c| c.original_name.clone().map(|o| (o, c.name.clone()))).collect()
}

fn renamed(cols: &[String], map: &HashMap<String, String>) -> Vec<String> {
    cols.iter().map(|c| map.get(c).cloned().unwrap_or_else(|| c.clone())).collect()
}

fn same_definition(a: &ColumnDesign, b: &ColumnDesign) -> bool {
    a.data_type.eq_ignore_ascii_case(&b.data_type)
        && a.nullable == b.nullable
        && a.default == b.default
        && a.auto_increment == b.auto_increment
        && a.comment == b.comment
        && a.primary_key == b.primary_key
}

/// Method an index gets: `Normal`/`Unique` default to BTREE (stated explicitly in MySQL DDL); FULLTEXT and
/// SPATIAL indexes have none.
fn index_method(i: &IndexDesign) -> Option<IndexMethod> {
    matches!(i.kind, IndexKind::Normal | IndexKind::Unique).then(|| i.method.unwrap_or(IndexMethod::Btree))
}

/// Whether index `o` of the original table still matches `n` once columns are renamed, so it can be kept.
fn same_index(o: &IndexDesign, n: &IndexDesign, map: &HashMap<String, String>) -> bool {
    let comment = |i: &IndexDesign| i.comment.clone().unwrap_or_default();
    renamed(&o.columns, map) == n.columns && o.kind == n.kind && index_method(o) == index_method(n) && comment(o) == comment(n)
}

fn list(cols: &[String], quote: fn(&str) -> String) -> String {
    cols.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ")
}

fn fk_clause(fk: &ForeignKeyDesign, quote: fn(&str) -> String) -> String {
    let mut s = format!(
        "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
        quote(&fk.name),
        list(&fk.columns, quote),
        quote(&fk.ref_table),
        list(&fk.ref_columns, quote)
    );
    if !fk.on_delete.is_empty() {
        s += &format!(" ON DELETE {}", fk.on_delete);
    }
    if !fk.on_update.is_empty() {
        s += &format!(" ON UPDATE {}", fk.on_update);
    }
    s
}

/// Validates `design` and returns it with every unnamed index named `idx_<columns>` (`idx_name`,
/// `idx_first_last`), suffixed `_2`, `_3`… when that name is already taken in the table.
fn prepare(design: &TableDesign) -> crate::Result<TableDesign> {
    if design.name.trim().is_empty() {
        return Err(crate::AppError::validation("The table needs a name"));
    }
    if design.columns.is_empty() {
        return Err(crate::AppError::validation("The table needs at least one column"));
    }
    let mut seen = std::collections::HashSet::new();
    for c in &design.columns {
        if c.name.trim().is_empty() || c.data_type.trim().is_empty() {
            return Err(crate::AppError::validation("Every column needs a name and a type"));
        }
        if !seen.insert(c.name.to_lowercase()) {
            return Err(crate::AppError::validation(format!("Duplicate column {}", c.name)));
        }
    }
    let mut d = design.clone();
    let mut taken: std::collections::HashSet<String> =
        d.indexes.iter().map(|i| i.name.trim().to_lowercase()).filter(|n| !n.is_empty()).collect();
    for index in &mut d.indexes {
        if index.columns.is_empty() {
            let label = if index.name.trim().is_empty() { "Every index" } else { index.name.as_str() };
            return Err(crate::AppError::validation(format!("{label} needs at least one column")));
        }
        if index.name.trim().is_empty() {
            let base = format!("idx_{}", index.columns.join("_"));
            let name = (1..)
                .map(|n| if n == 1 { base.clone() } else { format!("{base}_{n}") })
                .find(|n| !taken.contains(&n.to_lowercase()))
                .unwrap();
            taken.insert(name.to_lowercase());
            index.name = name;
        }
    }
    Ok(d)
}

// ---------------------------------------------------------------- MySQL / MariaDB

pub(crate) mod mysql {
    use super::*;
    use crate::mysql::quote_ident as q;

    pub fn literal(s: &str) -> String {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
    }

    fn column_def(c: &ColumnDesign) -> String {
        let mut s = format!("{} {}", q(&c.name), c.data_type);
        s += if c.nullable && !c.primary_key { " NULL" } else { " NOT NULL" };
        if let Some(d) = c.default.as_deref().filter(|d| !d.is_empty()) {
            s += &format!(" DEFAULT {d}");
        }
        if c.auto_increment {
            s += " AUTO_INCREMENT";
        }
        if let Some(comment) = c.comment.as_deref().filter(|d| !d.is_empty()) {
            s += &format!(" COMMENT {}", literal(comment));
        }
        s
    }

    fn index_clause(i: &IndexDesign) -> String {
        let kind = match i.kind {
            IndexKind::Normal => "",
            IndexKind::Unique => "UNIQUE ",
            IndexKind::Fulltext => "FULLTEXT ",
            IndexKind::Spatial => "SPATIAL ",
        };
        let mut s = format!("{kind}INDEX {} ({})", q(&i.name), list(&i.columns, q));
        // FULLTEXT and SPATIAL indexes have a fixed structure and reject USING.
        match index_method(i) {
            Some(IndexMethod::Btree) => s += " USING BTREE",
            Some(IndexMethod::Hash) => s += " USING HASH",
            None => {}
        }
        if let Some(comment) = i.comment.as_deref().filter(|c| !c.is_empty()) {
            s += &format!(" COMMENT {}", literal(comment));
        }
        s
    }

    fn options(o: &TableOptions, original: Option<&TableOptions>) -> Vec<String> {
        let changed = |new: &Option<String>, old: Option<&Option<String>>| match (new, old) {
            (Some(n), Some(Some(o))) => !n.eq_ignore_ascii_case(o),
            (Some(n), _) => !n.is_empty(),
            (None, _) => false,
        };
        let mut out = Vec::new();
        if changed(&o.engine, original.map(|x| &x.engine)) {
            out.push(format!("ENGINE={}", o.engine.as_deref().unwrap()));
        }
        if changed(&o.charset, original.map(|x| &x.charset)) {
            out.push(format!("DEFAULT CHARSET={}", o.charset.as_deref().unwrap()));
        }
        if changed(&o.collation, original.map(|x| &x.collation)) {
            out.push(format!("COLLATE={}", o.collation.as_deref().unwrap()));
        }
        let comment_changed = match original {
            Some(orig) => o.comment.as_deref().unwrap_or("") != orig.comment.as_deref().unwrap_or(""),
            None => o.comment.as_deref().is_some_and(|c| !c.is_empty()),
        };
        if comment_changed {
            out.push(format!("COMMENT={}", literal(o.comment.as_deref().unwrap_or(""))));
        }
        out
    }

    pub fn create(db: &str, d: &TableDesign) -> crate::Result<Vec<String>> {
        let d = &prepare(d)?;
        let mut parts: Vec<String> = d.columns.iter().map(column_def).collect();
        let pk = d.primary_key();
        if !pk.is_empty() {
            parts.push(format!("PRIMARY KEY ({})", list(&pk, q)));
        }
        parts.extend(d.indexes.iter().map(index_clause));
        parts.extend(d.foreign_keys.iter().map(|f| fk_clause(f, q)));
        let mut sql = format!("CREATE TABLE {}.{} (\n  {}\n)", q(db), q(&d.name), parts.join(",\n  "));
        let opts = options(&d.options, None);
        if !opts.is_empty() {
            sql += &format!(" {}", opts.join(" "));
        }
        Ok(vec![sql])
    }

    /// One `ALTER TABLE` statement, which MySQL applies atomically.
    pub fn alter(db: &str, original: &TableDesign, d: &TableDesign) -> crate::Result<Vec<String>> {
        let d = &prepare(d)?;
        let map = rename_map(d);
        let orig_cols: HashMap<&str, &ColumnDesign> = original.columns.iter().map(|c| (c.name.as_str(), c)).collect();
        let mut clauses = Vec::new();

        let new_fks: HashMap<&str, &ForeignKeyDesign> = d.foreign_keys.iter().map(|f| (f.name.as_str(), f)).collect();
        let fk_same = |o: &ForeignKeyDesign, n: &ForeignKeyDesign| {
            renamed(&o.columns, &map) == n.columns
                && o.ref_table == n.ref_table
                && o.ref_columns == n.ref_columns
                && o.on_update == n.on_update
                && o.on_delete == n.on_delete
        };
        for fk in &original.foreign_keys {
            if new_fks.get(fk.name.as_str()).is_none_or(|n| !fk_same(fk, n)) {
                clauses.push(format!("DROP FOREIGN KEY {}", q(&fk.name)));
            }
        }
        let new_idx: HashMap<&str, &IndexDesign> = d.indexes.iter().map(|i| (i.name.as_str(), i)).collect();
        let idx_same = |o: &IndexDesign, n: &IndexDesign| same_index(o, n, &map);
        for idx in &original.indexes {
            if new_idx.get(idx.name.as_str()).is_none_or(|n| !idx_same(idx, n)) {
                clauses.push(format!("DROP INDEX {}", q(&idx.name)));
            }
        }
        let kept: std::collections::HashSet<&str> = d.columns.iter().filter_map(|c| c.original_name.as_deref()).collect();
        for c in &original.columns {
            if !kept.contains(c.name.as_str()) {
                clauses.push(format!("DROP COLUMN {}", q(&c.name)));
            }
        }
        let orig_pk = renamed(&original.primary_key(), &map);
        let new_pk = d.primary_key();
        let pk_changed = orig_pk != new_pk;
        if pk_changed && !original.primary_key().is_empty() {
            clauses.push("DROP PRIMARY KEY".to_owned());
        }
        // Original order of the kept columns, to detect moves.
        let orig_order: Vec<&str> = original.columns.iter().map(|c| c.name.as_str()).filter(|n| kept.contains(n)).collect();
        for (i, c) in d.columns.iter().enumerate() {
            let position = match i {
                0 => " FIRST".to_owned(),
                _ => format!(" AFTER {}", q(&d.columns[i - 1].name)),
            };
            match &c.original_name {
                None => clauses.push(format!("ADD COLUMN {}{position}", column_def(c))),
                Some(o) => {
                    let oc = orig_cols.get(o.as_str()).copied();
                    let orig_prev = orig_order.iter().position(|n| n == o).and_then(|p| p.checked_sub(1)).map(|p| orig_order[p]);
                    let new_prev = i.checked_sub(1).map(|p| &d.columns[p]).map(|p| p.original_name.as_deref());
                    let moved = match new_prev {
                        None => orig_prev.is_some(),
                        Some(prev) => prev != orig_prev,
                    };
                    let changed = oc.is_none_or(|oc| !same_definition(oc, c)) || c.name != *o || moved;
                    if changed {
                        clauses.push(format!("CHANGE COLUMN {} {}{position}", q(o), column_def(c)));
                    }
                }
            }
        }
        if pk_changed && !new_pk.is_empty() {
            clauses.push(format!("ADD PRIMARY KEY ({})", list(&new_pk, q)));
        }
        let orig_idx: HashMap<&str, &IndexDesign> = original.indexes.iter().map(|i| (i.name.as_str(), i)).collect();
        for idx in &d.indexes {
            if orig_idx.get(idx.name.as_str()).is_none_or(|o| !idx_same(o, idx)) {
                clauses.push(format!("ADD {}", index_clause(idx)));
            }
        }
        let orig_fks: HashMap<&str, &ForeignKeyDesign> = original.foreign_keys.iter().map(|f| (f.name.as_str(), f)).collect();
        for fk in &d.foreign_keys {
            if orig_fks.get(fk.name.as_str()).is_none_or(|o| !fk_same(o, fk)) {
                clauses.push(format!("ADD {}", fk_clause(fk, q)));
            }
        }
        clauses.extend(options(&d.options, Some(&original.options)));
        if d.name != original.name {
            clauses.push(format!("RENAME TO {}.{}", q(db), q(&d.name)));
        }
        if clauses.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![format!("ALTER TABLE {}.{}\n  {}", q(db), q(&original.name), clauses.join(",\n  "))])
    }
}

// ---------------------------------------------------------------- SQLite

pub(crate) mod sqlite {
    use super::*;
    use crate::sqlite::quote_ident as q;

    /// Objects that depend on a table and must be recreated when it is rebuilt.
    pub struct Dependents {
        pub triggers: Vec<String>,
        pub views: Vec<(String, String)>,
    }

    pub const FK_OFF: &str = "PRAGMA foreign_keys = OFF";
    pub const FK_ON: &str = "PRAGMA foreign_keys = ON";

    fn column_def(c: &ColumnDesign, single_pk: bool) -> String {
        let mut s = format!("{} {}", q(&c.name), c.data_type);
        if c.primary_key && single_pk {
            s += " PRIMARY KEY";
            if c.auto_increment {
                s += " AUTOINCREMENT";
            }
        }
        if !c.nullable && !(c.primary_key && single_pk && c.data_type.eq_ignore_ascii_case("INTEGER")) {
            s += " NOT NULL";
        }
        if let Some(d) = c.default.as_deref().filter(|d| !d.is_empty()) {
            s += &format!(" DEFAULT {d}");
        }
        s
    }

    /// Unique constraints show up as `sqlite_autoindex_*`; recreated as named unique indexes.
    fn index_name(table: &str, i: &IndexDesign) -> String {
        if i.name.starts_with("sqlite_") { format!("uq_{table}_{}", i.columns.join("_")) } else { i.name.clone() }
    }

    fn create_index(db: &str, table: &str, i: &IndexDesign) -> crate::Result<String> {
        let unique = match i.kind {
            IndexKind::Normal => "",
            IndexKind::Unique => "UNIQUE ",
            IndexKind::Fulltext | IndexKind::Spatial => {
                return Err(crate::AppError::validation(format!("SQLite has no FULLTEXT or SPATIAL indexes ({})", i.name)));
            }
        };
        Ok(format!("CREATE {unique}INDEX {}.{} ON {} ({})", q(db), q(&index_name(table, i)), q(table), list(&i.columns, q)))
    }

    fn create_table_sql(db: &str, name: &str, d: &TableDesign) -> String {
        let pk = d.primary_key();
        let single = pk.len() == 1;
        let mut parts: Vec<String> = d.columns.iter().map(|c| column_def(c, single)).collect();
        if pk.len() > 1 {
            parts.push(format!("PRIMARY KEY ({})", list(&pk, q)));
        }
        parts.extend(d.foreign_keys.iter().map(|f| fk_clause(f, q)));
        format!("CREATE TABLE {}.{} (\n  {}\n)", q(db), q(name), parts.join(",\n  "))
    }

    pub fn create(db: &str, d: &TableDesign) -> crate::Result<Vec<String>> {
        let d = &prepare(d)?;
        let mut out = vec![create_table_sql(db, &d.name, d)];
        for i in &d.indexes {
            out.push(create_index(db, &d.name, i)?);
        }
        Ok(out)
    }

    pub fn alter(db: &str, original: &TableDesign, d: &TableDesign, deps: Dependents) -> crate::Result<Vec<String>> {
        let d = &prepare(d)?;
        let map = rename_map(d);
        let orig_cols: HashMap<&str, &ColumnDesign> = original.columns.iter().map(|c| (c.name.as_str(), c)).collect();
        let kept: Vec<&ColumnDesign> = d.columns.iter().filter(|c| c.original_name.is_some()).collect();
        let kept_in_original_order =
            original.columns.iter().filter(|c| kept.iter().any(|k| k.original_name.as_deref() == Some(&c.name))).count() == kept.len()
                && kept.iter().map(|c| c.original_name.as_deref().unwrap()).eq(original
                    .columns
                    .iter()
                    .map(|c| c.name.as_str())
                    .filter(|n| kept.iter().any(|k| k.original_name.as_deref() == Some(*n))));
        let dropped_any = original.columns.len() != kept.len();
        let defs_unchanged = kept.iter().all(|c| orig_cols.get(c.original_name.as_deref().unwrap()).is_some_and(|o| same_definition(o, c)));
        // New columns may only be appended, and must be addable by ALTER TABLE ADD COLUMN.
        let first_new = d.columns.iter().position(|c| c.original_name.is_none()).unwrap_or(d.columns.len());
        let appended_only = d.columns[first_new..].iter().all(|c| c.original_name.is_none());
        let addable = d.columns[first_new..].iter().all(|c| !c.primary_key && !c.auto_increment && (c.nullable || c.default.is_some()));
        let fks_same = original.foreign_keys.len() == d.foreign_keys.len()
            && original.foreign_keys.iter().zip(&d.foreign_keys).all(|(o, n)| {
                renamed(&o.columns, &map) == n.columns
                    && o.ref_table == n.ref_table
                    && o.ref_columns == n.ref_columns
                    && o.on_update == n.on_update
                    && o.on_delete == n.on_delete
            });
        let idx_same = |o: &IndexDesign, n: &IndexDesign| same_index(o, n, &map);
        let new_idx: HashMap<&str, &IndexDesign> = d.indexes.iter().map(|i| (i.name.as_str(), i)).collect();
        let orig_idx: HashMap<&str, &IndexDesign> = original.indexes.iter().map(|i| (i.name.as_str(), i)).collect();
        let dropped_indexes: Vec<&IndexDesign> =
            original.indexes.iter().filter(|i| new_idx.get(i.name.as_str()).is_none_or(|n| !idx_same(i, n))).collect();
        let added_indexes: Vec<&IndexDesign> =
            d.indexes.iter().filter(|i| orig_idx.get(i.name.as_str()).is_none_or(|o| !idx_same(o, i))).collect();
        let auto_index_touched = dropped_indexes.iter().any(|i| i.name.starts_with("sqlite_"));

        let simple =
            kept_in_original_order && !dropped_any && defs_unchanged && appended_only && addable && fks_same && !auto_index_touched;
        let table = q(&original.name);
        let schema = q(db);
        let mut out = Vec::new();
        if simple {
            for i in &dropped_indexes {
                out.push(format!("DROP INDEX {schema}.{}", q(&i.name)));
            }
            for c in &kept {
                let o = c.original_name.as_deref().unwrap();
                if c.name != o {
                    out.push(format!("ALTER TABLE {schema}.{table} RENAME COLUMN {} TO {}", q(o), q(&c.name)));
                }
            }
            for c in &d.columns[first_new..] {
                out.push(format!("ALTER TABLE {schema}.{table} ADD COLUMN {}", column_def(c, false)));
            }
            if d.name != original.name {
                out.push(format!("ALTER TABLE {schema}.{table} RENAME TO {}", q(&d.name)));
            }
            for i in &added_indexes {
                out.push(create_index(db, &d.name, i)?);
            }
            return Ok(out);
        }

        // Rebuild, following https://www.sqlite.org/lang_altertable.html#otheralter
        let tmp = format!("_schematadb_new_{}", d.name);
        out.push(FK_OFF.to_owned());
        for (name, _) in &deps.views {
            out.push(format!("DROP VIEW {schema}.{}", q(name)));
        }
        out.push(create_table_sql(db, &tmp, d));
        let new_cols: Vec<String> = kept.iter().map(|c| c.name.clone()).collect();
        let old_cols: Vec<String> = kept.iter().map(|c| c.original_name.clone().unwrap()).collect();
        if !kept.is_empty() {
            out.push(format!(
                "INSERT INTO {schema}.{} ({}) SELECT {} FROM {schema}.{table}",
                q(&tmp),
                list(&new_cols, q),
                list(&old_cols, q)
            ));
        }
        out.push(format!("DROP TABLE {schema}.{table}"));
        out.push(format!("ALTER TABLE {schema}.{} RENAME TO {}", q(&tmp), q(&d.name)));
        for i in &d.indexes {
            out.push(create_index(db, &d.name, i)?);
        }
        out.extend(deps.triggers);
        out.extend(deps.views.into_iter().map(|(_, sql)| sql));
        out.push(format!("PRAGMA {schema}.foreign_key_check"));
        out.push(FK_ON.to_owned());
        Ok(out)
    }
}
