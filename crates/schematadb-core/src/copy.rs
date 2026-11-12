//! Copying a table into another database, connection or engine.

use crate::sql_tokens::{Token, Tokens};
use crate::transfer::{
    BATCH, CancellationToken, ImportBatch, ImportOptions, Progress, ProgressFn, RowStream, StreamItem, hide_generated, open_read_session,
    quote_for, rename_create_table, select_all,
};
use crate::{AppError, CellValue, ColumnMeta, Driver, Engine, Result, StatementOutcome, TableInfo, Value};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where a copied table goes: possibly another database, connection or engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTableTarget {
    pub db: String,
    pub name: String,
    /// Copy the rows too, not only the structure.
    pub data: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTableReport {
    pub rows: u64,
}

/// Name of a database-scoped object (SQLite index, MySQL constraint) of `source` in its copy `name`:
/// `ix_orders_total` → `ix_orders_copy_total`, or prefixed with the copy's name when it does not mention the table.
fn copied_name(object: &str, source: &str, name: &str) -> String {
    if source == name {
        object.to_owned()
    } else if object.contains(source) {
        object.replacen(source, name, 1)
    } else {
        format!("{name}_{object}")
    }
}

/// Structure of `design` recreated as `name` in the target database (for copies into another engine). Self-references
/// follow the copy; foreign keys to tables missing from the target, table options of another engine, and index features
/// SQLite lacks (FULLTEXT/SPATIAL indexes, index methods and comments) are dropped. Index and foreign key names are
/// renamed after the copy (see [`copied_name`]).
pub(crate) fn copy_design(
    mut design: crate::TableDesign,
    name: &str,
    source_engine: Engine,
    target_engine: Engine,
    target_tables: &[String],
) -> crate::TableDesign {
    let source_name = std::mem::replace(&mut design.name, name.to_owned());
    design.foreign_keys.retain(|fk| fk.ref_table == source_name || target_tables.contains(&fk.ref_table));
    for fk in &mut design.foreign_keys {
        if fk.ref_table == source_name {
            fk.ref_table = name.to_owned();
        }
        fk.name = copied_name(&fk.name, &source_name, name);
    }
    for index in &mut design.indexes {
        index.name = copied_name(&index.name, &source_name, name);
    }
    if source_engine != target_engine {
        design.options = crate::TableOptions { comment: design.options.comment.take(), ..Default::default() };
    }
    if target_engine == Engine::Sqlite {
        // SQLite has neither FULLTEXT/SPATIAL indexes nor index methods and comments.
        design.indexes.retain(|i| matches!(i.kind, crate::IndexKind::Normal | crate::IndexKind::Unique));
        for index in &mut design.indexes {
            index.method = None;
            index.comment = None;
        }
    }
    for c in &mut design.columns {
        c.original_name = None;
        // SQLite only auto-increments an INTEGER PRIMARY KEY.
        if target_engine == Engine::Sqlite && c.auto_increment {
            c.data_type = "INTEGER".into();
        }
    }
    design
}

fn unexpected_definition(table: &str) -> AppError {
    AppError::validation(format!("The definition of {table} has a form Schemata cannot copy; copy the table with SQL"))
}

/// A `CONSTRAINT` line of a MySQL `SHOW CREATE TABLE` in the copy: renamed (constraint names are scoped to the database),
/// with a self-reference following the copy. `None` drops a foreign key whose referenced table `keep` rejects.
fn copy_constraint(definition: &str, source: &str, name: &str, keep: &impl Fn(Option<&str>, &str) -> bool) -> Option<String> {
    let q = crate::mysql::quote_ident;
    let tokens: Vec<Token> = Tokens::new(definition, Engine::MySql).collect();
    let Some(constraint) = tokens.first().filter(|t| t.is_keyword(definition, "CONSTRAINT")).and(tokens.get(1)) else {
        return Some(definition.to_owned());
    };
    let mut replacements = Vec::new();
    if let Some(constraint_name) = constraint.name(definition)
        && !["FOREIGN", "CHECK", "PRIMARY", "UNIQUE"].iter().any(|k| constraint.is_keyword(definition, k))
    {
        replacements.push((constraint.start, constraint.end, q(&copied_name(&constraint_name, source, name))));
    }
    if let Some(at) = tokens.iter().position(|t| t.is_keyword(definition, "REFERENCES")) {
        let first = tokens.get(at + 1)?;
        let (schema, table) = match (tokens.get(at + 2), tokens.get(at + 3)) {
            (Some(dot), Some(table)) if dot.is_symbol('.') => (first.name(definition), table),
            _ => (None, first),
        };
        let table_name = table.name(definition)?;
        if !keep(schema.as_deref(), &table_name) {
            return None;
        }
        if schema.is_none() && table_name == source {
            replacements.push((table.start, table.end, q(name)));
        }
    }
    let mut out = definition.to_owned();
    for (start, end, text) in replacements.into_iter().rev() {
        out.replace_range(start..end, &text);
    }
    Some(out)
}

/// The `SHOW CREATE TABLE` text of a MySQL/MariaDB table rewritten to create its copy `name`. Every column attribute
/// (ON UPDATE, charset and collation, generated and INVISIBLE columns), index and CHECK constraint is kept; constraints
/// are renamed after the copy, and foreign keys whose referenced table `keep(schema, table)` rejects are dropped.
fn mysql_copy_create(create_sql: &str, source: &str, name: &str, keep: impl Fn(Option<&str>, &str) -> bool) -> Result<String> {
    let lines: Vec<&str> = create_sql.lines().collect();
    let close = lines.iter().rposition(|l| l.starts_with(')')).filter(|i| *i > 0).ok_or_else(|| unexpected_definition(source))?;
    let head = rename_create_table(lines[0], &crate::mysql::quote_ident(name)).ok_or_else(|| unexpected_definition(source))?;
    let definitions: Vec<String> = lines[1..close]
        .iter()
        .filter_map(|line| {
            let line = line.trim_end();
            copy_constraint(line.strip_suffix(',').unwrap_or(line), source, name, &keep)
        })
        .collect();
    // The copy's AUTO_INCREMENT counter starts after its own rows.
    let mut options = lines[close].to_owned();
    if let Some(at) = options.find(" AUTO_INCREMENT=") {
        let end = options[at + 1..].find(' ').map_or(options.len(), |n| at + 1 + n);
        options.replace_range(at..end, "");
    }
    let rest = lines[close + 1..].iter().map(|l| format!("\n{l}")).collect::<String>();
    Ok(format!("{head}\n{}\n{options}{rest}", definitions.join(",\n")))
}

/// `CREATE [UNIQUE] INDEX [IF NOT EXISTS] [schema.]index ON table ...` with the index named `index_name(index)` and
/// the table replaced by `table`.
fn rename_create_index(sql: &str, index_name: impl Fn(&str) -> String, table: &str) -> Option<String> {
    let tokens: Vec<Token> = Tokens::new(sql, Engine::Sqlite).take(12).collect();
    let mut at = 0;
    // Consumes `word` when it comes next.
    let mut accept = |word: &str| {
        let found = tokens.get(at).is_some_and(|t| t.is_keyword(sql, word));
        if found {
            at += 1;
        }
        found
    };
    if !accept("CREATE") {
        return None;
    }
    accept("UNIQUE");
    if !accept("INDEX") || (accept("IF") && !(accept("NOT") && accept("EXISTS"))) {
        return None;
    }
    let start = tokens.get(at)?.start;
    if tokens.get(at + 1).is_some_and(|t| t.is_symbol('.')) {
        at += 2;
    }
    let index = tokens.get(at)?;
    tokens.get(at + 1).filter(|t| t.is_keyword(sql, "ON"))?;
    let table_token = tokens.get(at + 2)?;
    table_token.name(sql)?;
    let (before, between, after) = (&sql[..start], &sql[index.end..table_token.start], &sql[table_token.end..]);
    Some(format!("{before}{}{between}{table}{after}", index_name(&index.name(sql)?)))
}

/// Rewrites `REFERENCES source` (any letter case, quoted or not) to `REFERENCES replacement` in SQLite DDL.
fn rewrite_references(sql: &str, source: &str, replacement: &str) -> String {
    let tokens: Vec<Token> = Tokens::new(sql, Engine::Sqlite).collect();
    let mut out = sql.to_owned();
    for pair in tokens.windows(2).rev() {
        if pair[0].is_keyword(sql, "REFERENCES") && pair[1].name(sql).is_some_and(|n| n.eq_ignore_ascii_case(source)) {
            out.replace_range(pair[1].start..pair[1].end, replacement);
        }
    }
    out
}

/// Whether recreating a SQLite table from its design (columns, keys, plain indexes) keeps its whole definition.
fn sqlite_design_is_complete(info: &TableInfo, index_sql: &[String]) -> bool {
    const TABLE_FEATURES: [&str; 7] = ["COLLATE", "CHECK", "STRICT", "WITHOUT", "CONFLICT", "GENERATED", "AS"];
    const INDEX_FEATURES: [&str; 4] = ["COLLATE", "WHERE", "DESC", "ASC"];
    let mentions = |sql: &str, words: &[&str]| Tokens::new(sql, Engine::Sqlite).any(|t| words.iter().any(|w| t.is_keyword(sql, w)));
    // An expression index has parentheses inside its column list.
    let expression = |sql: &str| Tokens::new(sql, Engine::Sqlite).filter(|t| t.is_symbol('(')).count() > 1;
    !info.columns.iter().any(|c| c.generated)
        && !mentions(&info.create_sql, &TABLE_FEATURES)
        && index_sql.iter().all(|sql| !mentions(sql, &INDEX_FEATURES) && !expression(sql))
}

/// `CREATE INDEX` statements of a SQLite table (indexes behind UNIQUE and PRIMARY KEY constraints come with the table).
async fn sqlite_index_sql(source: &dyn Driver, db: &str, table: &str) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT sql FROM {}.sqlite_schema WHERE type = 'index' AND tbl_name = '{}' AND sql IS NOT NULL ORDER BY rowid",
        crate::sqlite::quote_ident(db),
        table.replace('\'', "''")
    );
    let result = source.execute_script(Some(db), &sql, 100_000, None).await?;
    if let Some(error) = result.error {
        return Err(error.error);
    }
    Ok(result
        .outcomes
        .into_iter()
        .flat_map(|o| match o {
            StatementOutcome::Rows { rows, .. } => rows,
            StatementOutcome::Affected { .. } => Vec::new(),
        })
        .filter_map(|row| match row.into_iter().next() {
            Some(CellValue::Text(sql)) => Some(sql),
            _ => None,
        })
        .collect())
}

/// DDL that recreates a table under the same engine from its own definition, so nothing the designer cannot represent
/// (column collations, CHECK constraints, ON UPDATE, generated and invisible columns, STRICT, WITHOUT ROWID, partial
/// and expression indexes) is lost. `None` when a SQLite design copy is complete and must drop foreign keys to tables
/// missing from the target.
async fn same_engine_ddl(
    source: &dyn Driver,
    source_db: &str,
    info: &TableInfo,
    target: &dyn Driver,
    to: &CopyTableTarget,
    target_tables: &[String],
) -> Result<Option<Vec<String>>> {
    match source.engine() {
        Engine::MySql | Engine::MariaDb => {
            let same_server = std::ptr::addr_eq(source, target);
            let keep = |schema: Option<&str>, table: &str| match schema {
                None => table == info.name || target_tables.iter().any(|t| t == table),
                // References into another database hold only on the same server.
                Some(_) => same_server,
            };
            Ok(Some(vec![mysql_copy_create(&info.create_sql, &info.name, &to.name, keep)?]))
        }
        Engine::Sqlite => {
            let q = crate::sqlite::quote_ident;
            let index_sql = sqlite_index_sql(source, source_db, &info.name).await?;
            let missing: Vec<&str> = info
                .foreign_keys
                .iter()
                .map(|f| f.ref_table.as_str())
                .filter(|t| !t.eq_ignore_ascii_case(&info.name) && !target_tables.iter().any(|x| x.eq_ignore_ascii_case(t)))
                .collect();
            if !missing.is_empty() {
                if sqlite_design_is_complete(info, &index_sql) {
                    return Ok(None);
                }
                return Err(AppError::validation(format!(
                    "{} cannot be copied to {} without changing its definition: its foreign keys reference {}, which {} not exist there. Copy {} first.",
                    info.name,
                    to.db,
                    missing.join(", "),
                    if missing.len() == 1 { "does" } else { "do" },
                    if missing.len() == 1 { "that table" } else { "those tables" },
                )));
            }
            let create = rename_create_table(&info.create_sql, &format!("{}.{}", q(&to.db), q(&to.name)))
                .ok_or_else(|| unexpected_definition(&info.name))?;
            let mut ddl = vec![rewrite_references(&create, &info.name, &q(&to.name))];
            for sql in &index_sql {
                let index = |index: &str| format!("{}.{}", q(&to.db), q(&copied_name(index, &info.name, &to.name)));
                ddl.push(rename_create_index(sql, index, &q(&to.name)).ok_or_else(|| unexpected_definition(&info.name))?);
            }
            Ok(Some(ddl))
        }
    }
}

/// Copies a table's structure (and optionally its rows) into another database, possibly on another
/// connection or engine. The new table is dropped again when the data copy fails or is cancelled.
/// Within one engine the table's own definition is copied; into another engine its design is translated, which drops
/// what that engine lacks (see [`copy_design`]).
pub async fn copy_table(
    source: &dyn Driver,
    source_db: &str,
    table: &str,
    target: &dyn Driver,
    to: &CopyTableTarget,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<CopyTableReport> {
    if to.name.trim().is_empty() {
        return Err(AppError::validation("Enter a name for the copy"));
    }
    let target_tables: Vec<String> = target.list_objects(&to.db).await?.into_iter().map(|o| o.name).collect();
    if target_tables.iter().any(|t| t.eq_ignore_ascii_case(&to.name)) {
        return Err(AppError::validation(format!("A table named {} already exists in {}", to.name, to.db)));
    }
    let info = source.table_info(source_db, table).await?;
    let own_definition = if source.engine() == target.engine() {
        same_engine_ddl(source, source_db, &info, target, to, &target_tables).await?
    } else {
        None
    };
    let ddl = match own_definition {
        Some(ddl) => ddl,
        None => {
            if let Some(column) = info.columns.iter().find(|c| c.generated) {
                return Err(AppError::validation(format!(
                    "{table} has the generated column {}, whose expression cannot be translated; copy it within the same engine or with SQL",
                    column.name
                )));
            }
            let design =
                copy_design(source.table_design(source_db, table).await?, &to.name, source.engine(), target.engine(), &target_tables);
            target.plan_table(&to.db, None, &design).await?
        }
    };
    target.apply_ddl(&to.db, &ddl).await?;
    if !to.data {
        return Ok(CopyTableReport { rows: 0 });
    }
    match copy_rows(source, source_db, &info, target, to, progress, cancel).await {
        Ok(rows) => Ok(CopyTableReport { rows }),
        Err(e) => {
            let _ = target.drop_object(&to.db, &to.name, crate::ObjectKind::Table).await;
            Err(e)
        }
    }
}

/// Copies the rows. Between MySQL/MariaDB servers they are read and written in UTC, so TIMESTAMPs keep their instant
/// whatever the time zones of the two sessions.
async fn copy_rows(
    source: &dyn Driver,
    source_db: &str,
    info: &TableInfo,
    target: &dyn Driver,
    to: &CopyTableTarget,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<u64> {
    if source.engine() == Engine::Sqlite || target.engine() == Engine::Sqlite {
        return read_and_copy(source, None, source_db, info, target, to, progress, cancel).await;
    }
    let session = format!("schemata-copy-{}", uuid::Uuid::new_v4());
    let copied = async {
        open_read_session(source, &session, source_db, false).await?;
        read_and_copy(source, Some(&session), source_db, info, target, to, progress, cancel).await
    }
    .await;
    let _ = source.close_editor_session(&session).await;
    copied
}

#[allow(clippy::too_many_arguments)]
async fn read_and_copy(
    source: &dyn Driver,
    session: Option<&str>,
    source_db: &str,
    info: &TableInfo,
    target: &dyn Driver,
    to: &CopyTableTarget,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<u64> {
    let total = Some(source.count_rows(source_db, &info.name, &[]).await?);
    let q = quote_for(source.engine());
    let sql = select_all(source.engine(), source_db, info);
    let mut stream = match session {
        Some(session) => source.stream_query_in_session(session, Some(source_db), &sql, BATCH).await?,
        None => source.stream_query(Some(source_db), &sql, BATCH).await?,
    };
    let mut columns = match stream.recv().await {
        Some(Ok(StreamItem::Columns(c))) => c,
        Some(Err(e)) => return Err(e),
        _ => return Err(AppError::sql("The table did not return a result set")),
    };
    // The copy computes its generated columns itself.
    hide_generated(&mut columns, info);
    let options = ImportOptions { utc_timestamps: session.is_some(), ..ImportOptions::default() };
    if source.engine() == Engine::Sqlite && std::ptr::addr_eq(source, target) {
        // A SQLite stream holds the driver's only connection. Drain it to a temporary database first;
        // columns without affinity retain integers, floats, text (including NULs) and BLOBs exactly.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("rows.sqlite");
        let staging = crate::connect(&crate::ConnectParams::sqlite(path.to_string_lossy().into_owned(), true)).await?;
        let names = columns.iter().filter(|c| !c.hidden).map(|c| q(&c.name)).collect::<Vec<_>>().join(", ");
        let created = staging.execute_script(Some("main"), &format!("CREATE TABLE staged_rows ({names})"), 1, None).await?;
        if let Some(error) = created.error {
            return Err(error.error);
        }
        let staged_target = CopyTableTarget { db: "main".into(), name: "staged_rows".into(), data: true };
        import_stream(stream, &columns, staging.as_ref(), &staged_target, options, Arc::new(|_| {}), cancel.clone(), total).await?;
        let mut staged_stream = staging.stream_query(Some("main"), "SELECT * FROM staged_rows", BATCH).await?;
        let staged_columns = match staged_stream.recv().await {
            Some(Ok(StreamItem::Columns(c))) => c,
            Some(Err(e)) => return Err(e),
            _ => return Err(AppError::sql("The staged table did not return a result set")),
        };
        return import_stream(staged_stream, &staged_columns, target, to, options, progress, cancel, total).await;
    }
    import_stream(stream, &columns, target, to, options, progress, cancel, total).await
}

#[allow(clippy::too_many_arguments)]
async fn import_stream(
    mut stream: RowStream,
    columns: &[ColumnMeta],
    target: &dyn Driver,
    to: &CopyTableTarget,
    options: ImportOptions,
    progress: ProgressFn,
    cancel: CancellationToken,
    total: Option<u64>,
) -> Result<u64> {
    let visible: Vec<usize> = (0..columns.len()).filter(|i| !columns[*i].hidden).collect();
    let names: Vec<String> = visible.iter().map(|i| columns[*i].name.clone()).collect();
    progress(Progress { done: 0, total });
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<ImportBatch>>(4);
    let feed = async move {
        let mut done = 0u64;
        loop {
            let item = tokio::select! {
                biased;
                _ = cancel.cancelled() => Some(Err(AppError::cancelled())),
                item = stream.recv() => item,
            };
            let rows = match item {
                None => break,
                Some(Ok(StreamItem::Columns(_))) => continue,
                Some(Ok(StreamItem::Rows(rows))) => rows,
                Some(Err(e)) => {
                    let _ = tx.send(Err(e)).await;
                    break;
                }
            };
            let batch: ImportBatch = rows
                .into_iter()
                .enumerate()
                .map(|(i, mut r)| (done + i as u64 + 1, visible.iter().map(|c| std::mem::replace(&mut r[*c], Value::Null)).collect()))
                .collect();
            done += batch.len() as u64;
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let _ = tx.send(Err(AppError::cancelled())).await;
                    break;
                }
                sent = tx.send(Ok(batch)) => {
                    if sent.is_err() {
                        break;
                    }
                }
            }
            progress(Progress { done, total });
        }
    };
    let (report, ()) = tokio::join!(target.import_rows(&to.db, &to.name, &names, rx, options), feed);
    let report = report?;
    if let Some(f) = report.failed.first() {
        return Err(AppError::sql(format!("Row {} could not be copied: {}", f.line, f.message)));
    }
    Ok(report.inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_copy_keeps_column_attributes_and_renames_constraints() {
        let create = "CREATE TABLE `orders` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  `code` varchar(10) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,\n  `changed` timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,\n  `parent` int DEFAULT NULL,\n  `customer` int DEFAULT NULL,\n  PRIMARY KEY (`id`),\n  KEY `fk_orders_customer` (`customer`),\n  CONSTRAINT `fk_orders_customer` FOREIGN KEY (`customer`) REFERENCES `customer` (`id`),\n  CONSTRAINT `fk_parent` FOREIGN KEY (`parent`) REFERENCES `orders` (`id`),\n  CONSTRAINT `orders_chk_1` CHECK ((`id` > 0))\n) ENGINE=InnoDB AUTO_INCREMENT=7 DEFAULT CHARSET=utf8mb4";
        let copy = mysql_copy_create(create, "orders", "orders_copy", |schema, table| schema.is_none() && table == "orders").unwrap();
        assert_eq!(
            copy,
            "CREATE TABLE `orders_copy` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  `code` varchar(10) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,\n  `changed` timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,\n  `parent` int DEFAULT NULL,\n  `customer` int DEFAULT NULL,\n  PRIMARY KEY (`id`),\n  KEY `fk_orders_customer` (`customer`),\n  CONSTRAINT `orders_copy_fk_parent` FOREIGN KEY (`parent`) REFERENCES `orders_copy` (`id`),\n  CONSTRAINT `orders_copy_chk_1` CHECK ((`id` > 0))\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4"
        );
    }

    #[test]
    fn sqlite_index_and_references_follow_the_copy() {
        let index = rename_create_index(
            "CREATE UNIQUE INDEX \"ix_items_code\" ON items (code COLLATE NOCASE) WHERE code IS NOT NULL",
            |name| format!("\"main\".\"{}\"", copied_name(name, "items", "items2")),
            "\"items2\"",
        );
        assert_eq!(
            index.as_deref(),
            Some("CREATE UNIQUE INDEX \"main\".\"ix_items2_code\" ON \"items2\" (code COLLATE NOCASE) WHERE code IS NOT NULL")
        );
        let table = "CREATE TABLE x (id INTEGER PRIMARY KEY, parent INT REFERENCES [Items] (id), note TEXT DEFAULT 'REFERENCES items')";
        assert_eq!(
            rewrite_references(table, "items", "\"items2\""),
            "CREATE TABLE x (id INTEGER PRIMARY KEY, parent INT REFERENCES \"items2\" (id), note TEXT DEFAULT 'REFERENCES items')"
        );
    }
}
