//! SQL text building shared by the drivers. Values are always returned as bind parameters.

use crate::{AppError, ChangeError, Filter, FilterOp, PageRequest, Result, RowChange, RowIdentity, Value};

/// Builds ` WHERE ...` (or an empty string) and its bind values.
pub(crate) fn where_clause(filters: &[Filter], quote: fn(&str) -> String) -> Result<(String, Vec<Value>)> {
    let mut parts = Vec::new();
    let mut binds = Vec::new();
    for f in filters {
        let col = quote(&f.column);
        let value = || -> Result<Value> {
            f.value.as_ref().ok_or_else(|| AppError::validation(format!("Filter on {} needs a value", f.column)))?.to_value()
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
    /// Binds of `exists_sql` and `ambiguity_sql`.
    pub key_binds: Vec<Value>,
    /// Counts (up to 2) the rows the statement's match finds. Set when rows are matched by value but some columns
    /// cannot be compared: two matches may then be different rows, and the change must be refused.
    pub ambiguity_sql: Option<String>,
}

/// SQL spelling of the parts of a change set that differ between engines.
pub(crate) struct ChangeDialect {
    /// Tail of an insert of only default values.
    pub empty_insert: &'static str,
    /// Equality that also matches NULL to NULL (`<=>`, `IS`).
    pub null_safe_eq: &'static str,
    /// Tail limiting an UPDATE/DELETE to one row, or empty when the engine has none.
    pub single_row: &'static str,
}

/// Where and how a change set is applied.
pub(crate) struct ChangeTarget<'a> {
    /// Qualified, quoted table name.
    pub table_ref: String,
    pub identity: &'a RowIdentity,
    pub quote: fn(&str) -> String,
    pub dialect: ChangeDialect,
    /// When rows are matched by value, these text columns are compared as UTF-8 bytes (MySQL/MariaDB), so
    /// case-insensitive or PAD SPACE collations cannot match another row. Converting the column first keeps non-UTF-8
    /// charsets (latin1, …) comparable with the UTF-8 parameter.
    pub byte_exact: Vec<String>,
    /// Quoted spelling of the rowid for [`RowIdentity::RowId`].
    pub rowid: Option<String>,
    /// Columns whose loaded values (`expected`) must still match before a keyed change runs; empty when the key
    /// alone identifies the row for good.
    pub expected_columns: Vec<String>,
    /// Whether matching by value compares every column, so rows that match are true duplicates.
    pub complete_match: bool,
}

/// Refuses a change set built against another identity than the table's current one (for example after its
/// primary key changed): its key values would address other rows.
pub(crate) fn check_identity(loaded: &RowIdentity, current: &RowIdentity) -> Result<(), ChangeError> {
    if loaded == current {
        return Ok(());
    }
    Err(AppError::validation("The table's key changed since the data was loaded; refresh the data and enter the changes again").into())
}

/// Renders a change set against `target`.
pub(crate) fn change_statements(changes: &[RowChange], target: &ChangeTarget<'_>) -> Result<Vec<ChangeStatement>, ChangeError> {
    let quote = target.quote;
    let dialect = &target.dialect;
    let (key_names, key_columns): (Vec<String>, Vec<String>) = match target.identity {
        RowIdentity::RowId => {
            let rowid = target.rowid.clone().ok_or_else(|| ChangeError::from(AppError::validation("This table has no usable rowid")))?;
            (vec![crate::ROWID_COLUMN.to_owned()], vec![rowid])
        }
        identity => {
            let names = identity.columns();
            let quoted = names.iter().map(|c| quote(c)).collect();
            (names, quoted)
        }
    };
    let needs_key = changes.iter().any(|c| !matches!(c, RowChange::Insert { .. }));
    if needs_key && key_columns.is_empty() {
        return Err(AppError::validation("This table has no primary or unique key, so its rows cannot be edited").into());
    }
    // Without a key, rows are matched by value: NULL must match NULL, and duplicates must not all change.
    let by_value = matches!(target.identity, RowIdentity::AllColumns { .. });
    let eq = if by_value { dialect.null_safe_eq } else { "=" };
    let limit = if by_value && !dialect.single_row.is_empty() { format!(" {}", dialect.single_row) } else { String::new() };
    let where_key = |key: &[crate::CellValue]| {
        key_names
            .iter()
            .zip(&key_columns)
            .zip(key)
            .map(|((name, c), value)| {
                if by_value && target.byte_exact.contains(name) {
                    format!("CAST(CONVERT({c} USING utf8mb4) AS BINARY) {eq} CAST(? AS BINARY)")
                } else if let crate::CellValue::Decimal(decimal) = value {
                    format!("{c} {eq} {}", decimal_parameter(decimal))
                } else {
                    format!("{c} {eq} ?")
                }
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let ambiguous = by_value && !target.complete_match;
    let table_ref = &target.table_ref;
    changes
        .iter()
        .enumerate()
        .map(|(index, change)| {
            let at = |error: AppError| ChangeError { index: Some(index), error };
            // The row's match: its key, plus its loaded values where the key alone is not trusted.
            let matcher = |key: &[crate::CellValue], expected: &[crate::ColumnValue]| -> Result<(String, Vec<Value>), ChangeError> {
                if key.len() != key_columns.len() {
                    return Err(at(AppError::validation("Row key does not match the table's identity")));
                }
                let mut binds: Vec<Value> = key.iter().map(|k| k.to_value().map_err(at)).collect::<Result<_, _>>()?;
                let mut sql = where_key(key);
                for column in &target.expected_columns {
                    // A truncated binary preview cannot be compared; the other columns still pin the row.
                    let Some(value) = expected.iter().find(|v| &v.column == column).and_then(|v| v.value.to_value().ok()) else { continue };
                    // Bytes compare as bytes: TEXT that is not valid UTF-8 is loaded as bytes (SQLite).
                    let lhs = if matches!(value, Value::Bytes(_)) { format!("CAST({} AS BLOB)", quote(column)) } else { quote(column) };
                    sql += &format!(" AND {lhs} {} ?", dialect.null_safe_eq);
                    binds.push(value);
                }
                Ok((sql, binds))
            };
            let ambiguity_sql = |where_sql: &str| {
                ambiguous.then(|| format!("SELECT COUNT(*) FROM (SELECT 1 FROM {table_ref} WHERE {where_sql} LIMIT 2) AS schematadb_match"))
            };
            Ok(match change {
                RowChange::Update { key, values, expected } => {
                    if values.is_empty() {
                        return Err(at(AppError::validation("Nothing to update")));
                    }
                    let sets = values.iter().map(|v| format!("{} = ?", quote(&v.column))).collect::<Vec<_>>().join(", ");
                    let mut binds = values.iter().map(|v| v.value.to_value().map_err(at)).collect::<Result<Vec<_>, _>>()?;
                    let (where_sql, key_binds) = matcher(key, expected)?;
                    binds.extend(key_binds.iter().cloned());
                    ChangeStatement {
                        sql: format!("UPDATE {table_ref} SET {sets} WHERE {where_sql}{limit}"),
                        binds,
                        keyed: true,
                        exists_sql: Some(format!("SELECT 1 FROM {table_ref} WHERE {where_sql}")),
                        ambiguity_sql: ambiguity_sql(&where_sql),
                        key_binds,
                    }
                }
                RowChange::Delete { key, expected } => {
                    let (where_sql, key_binds) = matcher(key, expected)?;
                    ChangeStatement {
                        sql: format!("DELETE FROM {table_ref} WHERE {where_sql}{limit}"),
                        binds: key_binds.clone(),
                        keyed: true,
                        exists_sql: None,
                        ambiguity_sql: ambiguity_sql(&where_sql),
                        key_binds,
                    }
                }
                RowChange::Insert { values } => {
                    let sql = if values.is_empty() {
                        format!("INSERT INTO {table_ref} {}", dialect.empty_insert)
                    } else {
                        let cols = values.iter().map(|v| quote(&v.column)).collect::<Vec<_>>().join(", ");
                        let marks = vec!["?"; values.len()].join(", ");
                        format!("INSERT INTO {table_ref} ({cols}) VALUES ({marks})")
                    };
                    let binds = values.iter().map(|v| v.value.to_value().map_err(at)).collect::<Result<Vec<_>, _>>()?;
                    ChangeStatement { sql, binds, keyed: false, exists_sql: None, key_binds: Vec::new(), ambiguity_sql: None }
                }
            })
        })
        .collect()
}

/// A DECIMAL parameter compared as a decimal of its own precision: compared with a string, MySQL and MariaDB convert
/// both sides to floating point, so nearby values match. Only MySQL/MariaDB read DECIMAL values.
fn decimal_parameter(value: &str) -> String {
    let digits = value.strip_prefix(['-', '+']).unwrap_or(value);
    let (int, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    let plain = !int.is_empty() && int.chars().chain(fraction.chars()).all(|c| c.is_ascii_digit());
    let (precision, scale) = (int.len() + fraction.len(), fraction.len());
    if plain && precision <= 65 && scale <= 30 { format!("CAST(? AS DECIMAL({precision},{scale}))") } else { "?".to_owned() }
}

pub(crate) fn vanished(index: usize) -> ChangeError {
    ChangeError {
        index: Some(index),
        error: AppError::sql("The row no longer exists or changed since it was loaded; refresh the data and try again"),
    }
}

/// Several rows match a row compared by value, and they may differ in columns that cannot be compared.
pub(crate) fn ambiguous(index: usize) -> ChangeError {
    ChangeError {
        index: Some(index),
        error: AppError::validation(
            "Several rows match this row's values and may differ only in columns that cannot be compared (binary, JSON, FLOAT, BIT or spatial). Nothing was saved; add a primary key or change these rows with SQL.",
        ),
    }
}

/// A keyed change hit more than one row; the transaction must be rolled back.
pub(crate) fn several(index: usize, affected: u64) -> ChangeError {
    ChangeError {
        index: Some(index),
        error: AppError::sql(format!("The change matched {affected} rows instead of one; nothing was saved")),
    }
}

/// Whether a statement only reads, so running it again (to load more rows or to export them) has no side effects.
/// Conservative: anything but a plain `SELECT`/`WITH`/`VALUES`/`TABLE`/`SHOW`/`DESCRIBE`/`EXPLAIN` is a write, and so is
/// one that mentions a writing keyword anywhere (`EXPLAIN ANALYZE DELETE …` runs the delete), a locking function or a
/// variable assignment. A `SELECT` that calls stored functions may still have side effects the server alone knows about.
pub fn is_read_only_query(sql: &str) -> bool {
    let mut rest = sql.trim_start();
    // Leading comments.
    loop {
        if let Some(r) = rest.strip_prefix("--").or_else(|| rest.strip_prefix('#')) {
            rest = r.split_once('\n').map_or("", |(_, tail)| tail).trim_start();
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.split_once("*/").map_or("", |(_, tail)| tail).trim_start();
        } else {
            break;
        }
    }
    let rest = rest.trim_start_matches(|c: char| c == '(' || c.is_whitespace());
    let first = rest.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').next().unwrap_or("").to_ascii_uppercase();
    let upper = sql.to_ascii_uppercase();
    let words: Vec<&str> = upper.split(|c: char| !c.is_ascii_alphanumeric() && c != '_').collect();
    let writes = [
        "INSERT",
        "UPDATE",
        "DELETE",
        "REPLACE",
        "INTO",
        "FOR",
        "RETURNING",
        "CALL",
        "LOCK",
        "NEXTVAL",
        "SETVAL",
        "GET_LOCK",
        "RELEASE_LOCK",
        "RELEASE_ALL_LOCKS",
    ]
    .iter()
    .any(|w| words.contains(w))
        || upper.contains(":=");
    match first.as_str() {
        "SELECT" | "WITH" | "VALUES" | "TABLE" | "DESCRIBE" | "DESC" | "EXPLAIN" => !writes,
        "SHOW" => true,
        _ => false,
    }
}

/// Wraps a SELECT so a window of its rows can be fetched again.
pub(crate) fn more_rows(statement: &str) -> String {
    let inner = statement.trim().trim_end_matches(';');
    format!("SELECT * FROM ({inner}) AS schematadb_more LIMIT ? OFFSET ?")
}
