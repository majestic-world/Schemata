//! SQL text building shared by the drivers. Values are always returned as bind parameters.

use crate::{AppError, ChangeError, Filter, FilterOp, PageRequest, ROWID_COLUMN, Result, RowChange, RowIdentity, Value};

/// Builds ` WHERE ...` (or an empty string) and its bind values.
pub(crate) fn where_clause(filters: &[Filter], quote: fn(&str) -> String) -> Result<(String, Vec<Value>)> {
    let mut parts = Vec::new();
    let mut binds = Vec::new();
    for f in filters {
        let col = quote(&f.column);
        let value = || -> Result<Value> {
            f.value
                .as_ref()
                .ok_or_else(|| AppError::validation(format!("Filter on {} needs a value", f.column)))?
                .to_value()
        };
        let like = |prefix: &str| -> Result<Value> {
            let text = value()?.as_text().unwrap_or_default();
            let escaped = text.replace('!', "!!").replace('%', "!%").replace('_', "!_");
            Ok(Value::Text(format!("{prefix}{escaped}%")))
        };
        let cmp = |op: &str| format!("{col} {op} ?");
        match f.op {
            FilterOp::Eq => parts.push(cmp("=")),
            FilterOp::NotEq => parts.push(cmp("<>")),
            FilterOp::Lt => parts.push(cmp("<")),
            FilterOp::Lte => parts.push(cmp("<=")),
            FilterOp::Gt => parts.push(cmp(">")),
            FilterOp::Gte => parts.push(cmp(">=")),
            FilterOp::Contains | FilterOp::StartsWith => parts.push(format!("{col} LIKE ? ESCAPE '!'")),
            FilterOp::IsNull => parts.push(format!("{col} IS NULL")),
            FilterOp::IsNotNull => parts.push(format!("{col} IS NOT NULL")),
        }
        match f.op {
            FilterOp::IsNull | FilterOp::IsNotNull => {}
            FilterOp::Contains => binds.push(like("%")?),
            FilterOp::StartsWith => binds.push(like("")?),
            _ => binds.push(value()?),
        }
    }
    let sql = if parts.is_empty() { String::new() } else { format!(" WHERE {}", parts.join(" AND ")) };
    Ok((sql, binds))
}

/// `SELECT <select_list> FROM <from> [WHERE] [ORDER BY] LIMIT ? OFFSET ?`.
pub(crate) fn select_page(select_list: &str, from: &str, req: &PageRequest, quote: fn(&str) -> String) -> Result<(String, Vec<Value>)> {
    let (where_sql, mut binds) = where_clause(&req.filters, quote)?;
    let order = match &req.sort {
        Some(s) => format!(" ORDER BY {} {}", quote(&s.column), if s.desc { "DESC" } else { "ASC" }),
        None => String::new(),
    };
    binds.push(Value::Int(req.limit as i64));
    binds.push(Value::Int(req.offset as i64));
    Ok((format!("SELECT {select_list} FROM {from}{where_sql}{order} LIMIT ? OFFSET ?"), binds))
}

pub(crate) fn count(from: &str, filters: &[Filter], quote: fn(&str) -> String) -> Result<(String, Vec<Value>)> {
    let (where_sql, binds) = where_clause(filters, quote)?;
    Ok((format!("SELECT COUNT(*) FROM {from}{where_sql}"), binds))
}

/// A change rendered as one parameterized statement.
pub(crate) struct ChangeStatement {
    pub sql: String,
    pub binds: Vec<Value>,
    /// Updates and deletes must hit exactly one existing row.
    pub keyed: bool,
    /// `SELECT 1` that checks the keyed row exists (MySQL reports 0 affected rows for no-op updates).
    pub exists_sql: Option<String>,
    pub key_binds: Vec<Value>,
}

/// Renders a change set against `table_ref`; `empty_insert` is the dialect's "insert defaults" tail.
pub(crate) fn change_statements(
    changes: &[RowChange],
    table_ref: &str,
    identity: &RowIdentity,
    quote: fn(&str) -> String,
    empty_insert: &str,
) -> Result<Vec<ChangeStatement>, ChangeError> {
    let key_columns: Vec<String> = identity
        .columns()
        .into_iter()
        .map(|c| if c == ROWID_COLUMN { "rowid".to_owned() } else { quote(&c) })
        .collect();
    let needs_key = changes.iter().any(|c| !matches!(c, RowChange::Insert { .. }));
    if needs_key && key_columns.is_empty() {
        return Err(AppError::validation("This table has no primary or unique key, so its rows cannot be edited").into());
    }
    let where_key = key_columns.iter().map(|c| format!("{c} = ?")).collect::<Vec<_>>().join(" AND ");
    changes
        .iter()
        .enumerate()
        .map(|(index, change)| {
            let at = |error: AppError| ChangeError { index: Some(index), error };
            let key_values = |key: &[crate::CellValue]| -> Result<Vec<Value>, ChangeError> {
                if key.len() != key_columns.len() {
                    return Err(at(AppError::validation("Row key does not match the table's identity")));
                }
                key.iter().map(|k| k.to_value().map_err(at)).collect()
            };
            Ok(match change {
                RowChange::Update { key, values } => {
                    if values.is_empty() {
                        return Err(at(AppError::validation("Nothing to update")));
                    }
                    let sets = values.iter().map(|v| format!("{} = ?", quote(&v.column))).collect::<Vec<_>>().join(", ");
                    let mut binds = values.iter().map(|v| v.value.to_value().map_err(at)).collect::<Result<Vec<_>, _>>()?;
                    let key_binds = key_values(key)?;
                    binds.extend(key_binds.iter().cloned());
                    ChangeStatement {
                        sql: format!("UPDATE {table_ref} SET {sets} WHERE {where_key}"),
                        binds,
                        keyed: true,
                        exists_sql: Some(format!("SELECT 1 FROM {table_ref} WHERE {where_key}")),
                        key_binds,
                    }
                }
                RowChange::Delete { key } => {
                    let key_binds = key_values(key)?;
                    ChangeStatement { sql: format!("DELETE FROM {table_ref} WHERE {where_key}"), binds: key_binds, keyed: true, exists_sql: None, key_binds: Vec::new() }
                }
                RowChange::Insert { values } => {
                    let sql = if values.is_empty() {
                        format!("INSERT INTO {table_ref} {empty_insert}")
                    } else {
                        let cols = values.iter().map(|v| quote(&v.column)).collect::<Vec<_>>().join(", ");
                        let marks = vec!["?"; values.len()].join(", ");
                        format!("INSERT INTO {table_ref} ({cols}) VALUES ({marks})")
                    };
                    let binds = values.iter().map(|v| v.value.to_value().map_err(at)).collect::<Result<Vec<_>, _>>()?;
                    ChangeStatement { sql, binds, keyed: false, exists_sql: None, key_binds: Vec::new() }
                }
            })
        })
        .collect()
}

pub(crate) fn vanished(index: usize) -> ChangeError {
    ChangeError { index: Some(index), error: AppError::sql("The row no longer exists; refresh the data and try again") }
}
