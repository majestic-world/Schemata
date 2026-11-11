use serde::{Deserialize, Serialize};

/// Largest integer magnitude a JavaScript number represents exactly (2^53).
const JS_SAFE_INT: i64 = 9_007_199_254_740_992;
/// Number of bytes shown in the hex preview of binary cells.
const BYTES_PREVIEW: usize = 32;

/// Full-fidelity value read from or written to a database.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    UInt(u64),
    Float(f64),
    Decimal(String),
    Text(String),
    Bytes(Vec<u8>),
    DateTime(String),
    Json(String),
}

/// Value representation exchanged with the UI. Precision-sensitive numbers travel as strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum CellValue {
    Null,
    Int(i64),
    BigInt(String),
    Float(f64),
    Decimal(String),
    Text(String),
    #[serde(rename_all = "camelCase")]
    Bytes {
        preview_hex: String,
        len: usize,
    },
    DateTime(String),
    Json(String),
}

impl Value {
    pub fn to_cell(&self) -> CellValue {
        match self {
            Value::Null => CellValue::Null,
            Value::Int(i) if i.unsigned_abs() <= JS_SAFE_INT as u64 => CellValue::Int(*i),
            Value::Int(i) => CellValue::BigInt(i.to_string()),
            Value::UInt(u) if *u <= JS_SAFE_INT as u64 => CellValue::Int(*u as i64),
            Value::UInt(u) => CellValue::BigInt(u.to_string()),
            Value::Float(f) if f.is_finite() => CellValue::Float(*f),
            Value::Float(f) => CellValue::Text(f.to_string()),
            Value::Decimal(d) => CellValue::Decimal(d.clone()),
            Value::Text(t) => CellValue::Text(t.clone()),
            Value::Bytes(b) => {
                CellValue::Bytes { preview_hex: b.iter().take(BYTES_PREVIEW).map(|x| format!("{x:02X}")).collect(), len: b.len() }
            }
            Value::DateTime(d) => CellValue::DateTime(d.clone()),
            Value::Json(j) => CellValue::Json(j.clone()),
        }
    }

    /// Text form used by CSV export and display; `None` for NULL.
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Null => None,
            Value::Int(i) => Some(i.to_string()),
            Value::UInt(u) => Some(u.to_string()),
            Value::Float(f) => Some(f.to_string()),
            Value::Decimal(s) | Value::Text(s) | Value::DateTime(s) | Value::Json(s) => Some(s.clone()),
            Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
        }
    }
}

impl CellValue {
    /// Converts a UI value into a bindable value. Binary previews cannot be written back.
    pub fn to_value(&self) -> crate::Result<Value> {
        Ok(match self {
            CellValue::Null => Value::Null,
            CellValue::Int(i) => Value::Int(*i),
            CellValue::BigInt(s) => {
                if let Ok(i) = s.parse::<i64>() {
                    Value::Int(i)
                } else if let Ok(u) = s.parse::<u64>() {
                    Value::UInt(u)
                } else {
                    return Err(crate::AppError::validation(format!("Invalid integer: {s}")));
                }
            }
            CellValue::Float(f) => Value::Float(*f),
            CellValue::Decimal(s) => Value::Decimal(s.clone()),
            CellValue::Text(s) => Value::Text(s.clone()),
            CellValue::DateTime(s) => Value::DateTime(s.clone()),
            CellValue::Json(s) => Value::Json(s.clone()),
            // A preview that covers the whole value is the value itself.
            CellValue::Bytes { preview_hex, len } if preview_hex.len() == len * 2 => Value::Bytes(
                (0..*len)
                    .map(|i| u8::from_str_radix(&preview_hex[i * 2..i * 2 + 2], 16))
                    .collect::<Result<_, _>>()
                    .map_err(|_| crate::AppError::validation("Invalid binary value"))?,
            ),
            CellValue::Bytes { .. } => {
                return Err(crate::AppError::validation("Binary values cannot be edited in the grid"));
            }
        })
    }
}
