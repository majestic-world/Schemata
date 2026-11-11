//! SQL text building shared by the drivers. Values are always returned as bind parameters.

use crate::{AppError, Filter, FilterOp, PageRequest, Result, Value};

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
