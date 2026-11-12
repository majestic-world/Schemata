//! Test data generation (Navicat's "Data Generation"): realistic values chosen per column from its name,
//! type and constraints, inserted in batches inside one transaction.

use crate::transfer::{BATCH, CancellationToken, ImportBatch, Progress, ProgressFn, StreamItem, quote_for};
use crate::{AppError, ColumnInfo, ColumnMeta, Driver, Page, Result, TableInfo, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// How the values of one column are produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GeneratorKind {
    /// Left out of the INSERT: the database fills it (auto-increment, defaults).
    Skip,
    Null,
    /// Consecutive integers after the column's current maximum.
    Sequence,
    Integer,
    Decimal,
    Boolean,
    FirstName,
    LastName,
    FullName,
    Email,
    Username,
    Phone,
    Company,
    City,
    State,
    Country,
    Address,
    ZipCode,
    Url,
    Uuid,
    IpAddress,
    Word,
    /// Product-like name, e.g. "Ergonomic Steel Chair".
    Title,
    Sentence,
    Paragraph,
    Slug,
    /// Short reference such as `KXQ-4821`.
    Code,
    Color,
    /// 64 hex characters (looks like a SHA-256 / password hash).
    Hash,
    Date,
    DateTime,
    Time,
    Year,
    /// One of the values of an ENUM/SET column.
    EnumValue,
    Json,
    Bytes,
    /// An existing value of the referenced column.
    ForeignKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnGenerator {
    pub column: String,
    pub kind: GeneratorKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateSpec {
    pub rows: u64,
    pub columns: Vec<ColumnGenerator>,
    /// Delete every existing row first.
    pub empty_first: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerateReport {
    pub rows: u64,
}

/// Upper bound on generated rows per run.
pub const MAX_GENERATED_ROWS: u64 = 1_000_000;

// ---------------------------------------------------------------- suggestions

fn has_word(name: &str, words: &[&str]) -> bool {
    name.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| words.contains(&part))
}

fn single_column_fk<'a>(info: &'a TableInfo, column: &str) -> Option<&'a crate::ForeignKeyInfo> {
    info.foreign_keys.iter().find(|fk| fk.columns.len() == 1 && fk.columns[0] == column)
}

/// Picks a generator from the column's name, type and constraints.
pub fn suggest_generator(info: &TableInfo, c: &ColumnInfo) -> GeneratorKind {
    use GeneratorKind as K;
    let t = SqlType::parse(&c.data_type);
    let name = c.name.to_ascii_lowercase();
    let base = t.base.as_str();
    if c.auto_increment || c.generated {
        return K::Skip;
    }
    if single_column_fk(info, &c.name).is_some() {
        return K::ForeignKey;
    }
    if base == "enum" || base == "set" {
        return K::EnumValue;
    }
    if base == "bool"
        || base == "boolean"
        || (base == "tinyint" && t.args.first().map(String::as_str) == Some("1"))
        || (base == "bit" && t.max_len() == Some(1))
    {
        return K::Boolean;
    }
    if base == "json" {
        return K::Json;
    }
    if base.contains("blob") || base.contains("binary") {
        return K::Bytes;
    }
    if base == "date" {
        return K::Date;
    }
    if base == "datetime" || base == "timestamp" {
        return K::DateTime;
    }
    if base == "time" {
        return K::Time;
    }
    if base == "year" {
        return K::Year;
    }
    if base == "uuid" || has_word(&name, &["uuid", "guid"]) {
        return K::Uuid;
    }
    if t.is_integer() {
        if c.primary_key && info.columns.iter().filter(|x| x.primary_key).count() == 1 {
            return K::Sequence;
        }
        if has_word(&name, &["is", "has", "active", "enabled", "deleted", "visible", "published", "verified"]) && !name.ends_with("_count")
        {
            return K::Boolean;
        }
        if has_word(&name, &["year"]) {
            return K::Year;
        }
        return K::Integer;
    }
    if t.is_decimal() {
        return K::Decimal;
    }
    if has_word(&name, &["date", "birthday", "dob"]) || name.ends_with("_on") {
        return K::Date;
    }
    if name.ends_with("_at") {
        return K::DateTime;
    }
    let words: &[(&[&str], K)] = &[
        (&["email", "mail"], K::Email),
        (&["first", "firstname", "fname", "given"], K::FirstName),
        (&["last", "lastname", "lname", "surname", "family"], K::LastName),
        (&["username", "login", "nick", "nickname", "handle"], K::Username),
        (&["phone", "tel", "telephone", "mobile", "cell", "fax", "whatsapp"], K::Phone),
        (&["company", "organization", "organisation", "org", "business", "employer", "brand"], K::Company),
        (&["city", "town"], K::City),
        (&["state", "province", "region"], K::State),
        (&["country", "nation"], K::Country),
        (&["address", "street", "addr", "address1", "address2"], K::Address),
        (&["zip", "zipcode", "postal", "postcode", "cep"], K::ZipCode),
        (&["url", "website", "site", "link", "homepage", "image", "avatar", "photo", "picture", "logo", "thumbnail"], K::Url),
        (&["ip"], K::IpAddress),
        (&["slug"], K::Slug),
        (&["color", "colour"], K::Color),
        (&["password", "pass", "hash", "token", "secret", "digest"], K::Hash),
        (&["code", "sku", "ref", "reference", "serial"], K::Code),
        (
            &[
                "description",
                "notes",
                "note",
                "body",
                "content",
                "bio",
                "about",
                "summary",
                "comment",
                "comments",
                "message",
                "details",
                "text",
            ],
            K::Paragraph,
        ),
        (&["title", "subject", "headline", "label", "product", "item"], K::Title),
    ];
    for (keys, kind) in words {
        if has_word(&name, keys) {
            return if *kind == K::Paragraph && t.max_len().is_some_and(|n| n < 200) { K::Sentence } else { *kind };
        }
    }
    if has_word(&name, &["name", "fullname", "customer", "author", "contact", "owner", "person"]) {
        // "name" in a people table is a person; elsewhere (products, categories…) it reads better as a title.
        let people = has_word(
            &info.name.to_ascii_lowercase(),
            &[
                "user",
                "users",
                "customer",
                "customers",
                "person",
                "people",
                "employee",
                "employees",
                "author",
                "authors",
                "contact",
                "contacts",
                "member",
                "members",
                "client",
                "clients",
                "student",
                "students",
            ],
        );
        return if people || name != "name" { K::FullName } else { K::Title };
    }
    if t.max_len() == Some(36) {
        return K::Uuid;
    }
    match t.max_len() {
        Some(n) if n <= 20 => K::Word,
        Some(n) if n <= 255 => K::Sentence,
        _ if base.contains("text") || base == "clob" => K::Paragraph,
        _ => K::Sentence,
    }
}

pub fn suggest_generators(info: &TableInfo) -> Vec<ColumnGenerator> {
    info.columns
        .iter()
        .map(|c| ColumnGenerator {
            column: c.name.clone(),
            kind: if c.generated { GeneratorKind::Skip } else { suggest_generator(info, c) },
        })
        .collect()
}

// ---------------------------------------------------------------- types

/// `decimal(10,2) unsigned` split into its parts (lowercase).
struct SqlType {
    base: String,
    args: Vec<String>,
    unsigned: bool,
}

impl SqlType {
    fn parse(data_type: &str) -> SqlType {
        let lower = data_type.trim().to_ascii_lowercase();
        let (head, args) = match (lower.find('('), lower.rfind(')')) {
            (Some(open), Some(close)) if close > open => {
                (format!("{} {}", &lower[..open], &lower[close + 1..]), split_args(&data_type.trim()[open + 1..close]))
            }
            _ => (lower.clone(), Vec::new()),
        };
        let mut words = head.split_whitespace();
        let base = words.next().unwrap_or("").to_owned();
        let unsigned = words.any(|w| w == "unsigned");
        SqlType { base, args, unsigned }
    }

    fn is_integer(&self) -> bool {
        self.base.contains("int") || self.base == "bit"
    }

    fn is_decimal(&self) -> bool {
        ["decimal", "numeric", "float", "double", "real", "money"].iter().any(|b| self.base.contains(b))
    }

    /// Declared length of character and binary types.
    fn max_len(&self) -> Option<usize> {
        let sized = ["char", "varchar", "binary", "varbinary", "nchar", "nvarchar", "character", "bit"];
        if sized.iter().any(|b| self.base == *b) || self.base.ends_with("char") {
            return self.args.first().and_then(|a| a.trim().parse().ok());
        }
        match self.base.as_str() {
            "tinytext" => Some(255),
            "text" => Some(65_535),
            _ => None,
        }
    }

    /// Largest value the integer type holds.
    fn int_max(&self) -> i64 {
        let (signed, unsigned) = match self.base.as_str() {
            "tinyint" => (127, 255),
            "smallint" => (32_767, 65_535),
            "mediumint" => (8_388_607, 16_777_215),
            "int" | "integer" => (2_147_483_647, 4_294_967_295),
            "bit" => (1, 1),
            _ => (i64::MAX, i64::MAX),
        };
        if self.unsigned { unsigned } else { signed }
    }

    /// `(largest value, scale)` of a fixed or floating point type.
    fn decimal_bounds(&self) -> (f64, usize) {
        let precision: Option<u32> = self.args.first().and_then(|a| a.trim().parse().ok());
        let scale: Option<usize> = self.args.get(1).and_then(|a| a.trim().parse().ok());
        match (precision, scale) {
            (Some(p), Some(s)) if p as usize > s => (10f64.powi((p as usize - s) as i32) - 1.0, s),
            (Some(p), None) if self.base == "decimal" || self.base == "numeric" => (10f64.powi(p as i32) - 1.0, 0),
            _ => (f64::MAX, 2),
        }
    }
}

/// `'a','b,c'` → `["a", "b,c"]`; also splits plain `10,2`.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if quoted && chars.peek() == Some(&'\'') => {
                cur.push('\'');
                chars.next();
            }
            '\'' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_owned()),
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() || !out.is_empty() {
        out.push(cur.trim().to_owned());
    }
    out
}

// ---------------------------------------------------------------- word lists

const FIRST_NAMES: &[&str] = &[
    "James",
    "Mary",
    "John",
    "Patricia",
    "Robert",
    "Jennifer",
    "Michael",
    "Linda",
    "William",
    "Elizabeth",
    "David",
    "Barbara",
    "Richard",
    "Susan",
    "Joseph",
    "Jessica",
    "Thomas",
    "Sarah",
    "Charles",
    "Karen",
    "Daniel",
    "Nancy",
    "Matthew",
    "Lisa",
    "Anthony",
    "Betty",
    "Mark",
    "Sandra",
    "Lucas",
    "Ana",
    "Pedro",
    "Julia",
    "Gabriel",
    "Beatriz",
    "Rafael",
    "Camila",
    "Mateus",
    "Larissa",
    "Sofia",
    "Miguel",
    "Laura",
    "Diego",
];
const LAST_NAMES: &[&str] = &[
    "Smith",
    "Johnson",
    "Williams",
    "Brown",
    "Jones",
    "Garcia",
    "Miller",
    "Davis",
    "Rodriguez",
    "Martinez",
    "Hernandez",
    "Lopez",
    "Gonzalez",
    "Wilson",
    "Anderson",
    "Thomas",
    "Taylor",
    "Moore",
    "Jackson",
    "Martin",
    "Lee",
    "Thompson",
    "White",
    "Harris",
    "Clark",
    "Lewis",
    "Silva",
    "Santos",
    "Oliveira",
    "Souza",
    "Costa",
    "Pereira",
    "Almeida",
    "Ferreira",
    "Carvalho",
    "Ribeiro",
    "Walker",
    "Young",
    "King",
    "Wright",
];
const CITIES: &[&str] = &[
    "New York",
    "Los Angeles",
    "Chicago",
    "Houston",
    "Phoenix",
    "Seattle",
    "Denver",
    "Boston",
    "Austin",
    "Portland",
    "London",
    "Paris",
    "Berlin",
    "Madrid",
    "Lisbon",
    "Rome",
    "Toronto",
    "Sydney",
    "Tokyo",
    "São Paulo",
    "Rio de Janeiro",
    "Curitiba",
    "Recife",
    "Buenos Aires",
    "Dublin",
];
const STATES: &[&str] = &[
    "California",
    "Texas",
    "Florida",
    "New York",
    "Washington",
    "Oregon",
    "Colorado",
    "Georgia",
    "Ohio",
    "Michigan",
    "Arizona",
    "Nevada",
    "São Paulo",
    "Minas Gerais",
    "Paraná",
    "Bahia",
    "Ontario",
    "Quebec",
    "Bavaria",
    "Catalonia",
];
const COUNTRIES: &[&str] = &[
    "United States",
    "Brazil",
    "Canada",
    "Mexico",
    "Argentina",
    "United Kingdom",
    "Germany",
    "France",
    "Spain",
    "Portugal",
    "Italy",
    "Netherlands",
    "Ireland",
    "Japan",
    "Australia",
    "India",
    "Chile",
    "Colombia",
    "Sweden",
    "Norway",
];
const STREETS: &[&str] = &[
    "Main",
    "Oak",
    "Pine",
    "Maple",
    "Cedar",
    "Elm",
    "Washington",
    "Lake",
    "Hill",
    "Park",
    "Sunset",
    "River",
    "Church",
    "Highland",
    "Mill",
];
const STREET_SUFFIXES: &[&str] = &["St", "Ave", "Rd", "Blvd", "Lane", "Drive", "Way", "Court"];
const COMPANY_SUFFIXES: &[&str] = &["Inc", "LLC", "Group", "Labs", "Systems", "Solutions", "Partners", "Industries", "Co", "Technologies"];
const DOMAINS: &[&str] = &["example.com", "mail.com", "test.org", "inbox.net", "demo.io", "sample.dev"];
const TLDS: &[&str] = &["com", "net", "org", "io", "dev", "co"];
const ADJECTIVES: &[&str] = &[
    "Ergonomic",
    "Rustic",
    "Sleek",
    "Small",
    "Large",
    "Practical",
    "Handcrafted",
    "Refined",
    "Modern",
    "Classic",
    "Gorgeous",
    "Intelligent",
    "Durable",
    "Lightweight",
    "Premium",
    "Compact",
    "Elegant",
    "Generic",
    "Fantastic",
    "Incredible",
];
const MATERIALS: &[&str] =
    &["Steel", "Wooden", "Cotton", "Plastic", "Granite", "Rubber", "Leather", "Bronze", "Glass", "Silk", "Concrete", "Marble"];
const NOUNS: &[&str] = &[
    "Chair",
    "Table",
    "Lamp",
    "Keyboard",
    "Shoes",
    "Shirt",
    "Gloves",
    "Hat",
    "Bike",
    "Wallet",
    "Watch",
    "Backpack",
    "Mug",
    "Bottle",
    "Desk",
    "Sofa",
    "Speaker",
    "Headphones",
    "Jacket",
    "Towels",
];
const LOREM: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "enim",
    "ad",
    "minim",
    "veniam",
    "quis",
    "nostrud",
    "exercitation",
    "ullamco",
    "laboris",
    "nisi",
    "aliquip",
    "ex",
    "ea",
    "commodo",
    "consequat",
    "duis",
    "aute",
    "irure",
    "in",
    "reprehenderit",
    "voluptate",
    "velit",
    "esse",
    "cillum",
    "fugiat",
    "nulla",
    "pariatur",
    "excepteur",
    "sint",
    "occaecat",
    "cupidatat",
    "non",
    "proident",
    "sunt",
    "culpa",
    "qui",
    "officia",
    "deserunt",
    "mollit",
    "anim",
    "id",
    "est",
    "laborum",
];

// ---------------------------------------------------------------- generation

struct ColumnPlan {
    name: String,
    kind: GeneratorKind,
    sql_type: SqlType,
    max_len: Option<usize>,
    /// Inclusive integer range.
    int_range: (i64, i64),
    /// `(min, max, scale)` for decimals.
    decimal: (f64, f64, usize),
    enum_values: Vec<String>,
    unique: bool,
    next_seq: i64,
    references: Vec<Value>,
    seen: HashSet<String>,
}

/// Realistic integer range for a column, within what its type holds.
fn int_range(name: &str, t: &SqlType) -> (i64, i64) {
    let (lo, hi) = if has_word(name, &["age"]) {
        (18, 90)
    } else if has_word(name, &["rating", "stars", "score"]) {
        (1, 5)
    } else if has_word(name, &["quantity", "qty", "stock", "count", "amount", "total", "units"]) {
        (0, 500)
    } else if has_word(name, &["price", "cost", "salary"]) {
        (1, 5000)
    } else {
        (1, 1000)
    };
    let max = t.int_max();
    (lo.min(max), hi.min(max))
}

fn decimal_range(name: &str, t: &SqlType) -> (f64, f64, usize) {
    let (max_value, scale) = t.decimal_bounds();
    let (lo, hi): (f64, f64) = if has_word(name, &["lat", "latitude"]) {
        (-90.0, 90.0)
    } else if has_word(name, &["lng", "lon", "long", "longitude"]) {
        (-180.0, 180.0)
    } else if has_word(name, &["rating", "score"]) {
        (1.0, 5.0)
    } else if has_word(name, &["discount", "rate", "percent", "percentage", "tax"]) {
        (0.0, 100.0)
    } else if has_word(name, &["weight", "height", "width", "length", "size"]) {
        (0.1, 100.0)
    } else {
        (1.0, 1000.0)
    };
    let lo = if t.unsigned { lo.max(0.0) } else { lo };
    (lo.min(max_value), hi.min(max_value), scale.min(6))
}

async fn scalar_values(driver: &dyn Driver, db: &str, sql: &str) -> Result<Vec<Value>> {
    let mut stream = driver.stream_query(Some(db), sql, BATCH).await?;
    let mut out = Vec::new();
    while let Some(item) = stream.recv().await {
        if let StreamItem::Rows(rows) = item? {
            out.extend(rows.into_iter().filter_map(|mut r| (!r.is_empty()).then(|| r.swap_remove(0))));
        }
    }
    Ok(out)
}

/// Resolves every chosen generator against the table: limits from the column type, the next sequence value,
/// existing values of referenced columns.
async fn plan(
    driver: &dyn Driver,
    db: &str,
    info: &TableInfo,
    columns: &[ColumnGenerator],
    empty_first: bool,
    rows: u64,
) -> Result<Vec<ColumnPlan>> {
    let q = quote_for(driver.engine());
    let single_pk: Vec<&ColumnInfo> = info.columns.iter().filter(|c| c.primary_key).collect();
    let mut plans = Vec::new();
    for g in columns.iter().filter(|g| g.kind != GeneratorKind::Skip) {
        let c = info
            .columns
            .iter()
            .find(|c| c.name == g.column)
            .ok_or_else(|| AppError::validation(format!("Column {} is not in {}", g.column, info.name)))?;
        info.validate_writable_columns([c.name.as_str()])?;
        let sql_type = SqlType::parse(&c.data_type);
        let lower = c.name.to_ascii_lowercase();
        let unique = (single_pk.len() == 1 && single_pk[0].name == c.name)
            || info.indexes.iter().any(|i| i.kind == crate::IndexKind::Unique && i.columns.len() == 1 && i.columns[0] == c.name);
        let mut next_seq: Option<i64> = Some(1);
        if !empty_first && (g.kind == GeneratorKind::Sequence || (unique && g.kind == GeneratorKind::Integer)) {
            let max = scalar_values(driver, db, &format!("SELECT MAX({}) FROM {}.{}", q(&c.name), q(db), q(&info.name))).await?;
            next_seq = match max.first() {
                Some(Value::Int(i)) => i.checked_add(1),
                Some(Value::UInt(u)) => i64::try_from(*u).ok().and_then(|i| i.checked_add(1)),
                Some(v) => match v.as_text().and_then(|t| t.parse::<f64>().ok()) {
                    // `as` saturates; values at the i64 limits cannot continue a sequence.
                    Some(f) if f >= i64::MAX as f64 || f < i64::MIN as f64 => None,
                    Some(f) => (f as i64).checked_add(1),
                    None => Some(1),
                },
                None => Some(1),
            };
        }
        // Every generated row takes the next value; the last one must still fit.
        let next_seq = next_seq.filter(|n| i64::try_from(rows).ok().and_then(|r| n.checked_add(r)).is_some()).ok_or_else(|| {
            AppError::validation(format!("The sequence for {} would exceed the largest integer value", c.name))
        })?;
        let mut references = Vec::new();
        if g.kind == GeneratorKind::ForeignKey {
            let fk = single_column_fk(info, &c.name)
                .ok_or_else(|| AppError::validation(format!("Column {} has no foreign key to take values from", c.name)))?;
            if !empty_first || fk.ref_table != info.name {
                references = scalar_values(
                    driver,
                    db,
                    &format!(
                        "SELECT DISTINCT {} FROM {}.{} WHERE {0} IS NOT NULL LIMIT 1000",
                        q(fk.ref_columns.first().filter(|_| fk.ref_columns.len() == 1).ok_or_else(|| {
                            AppError::validation(format!("The foreign key on {} has no single referenced column", c.name))
                        })?),
                        q(db),
                        q(&fk.ref_table)
                    ),
                )
                .await?;
            }
            if references.is_empty() && !c.nullable {
                return Err(AppError::validation(format!(
                    "{} references {}, which has no rows yet. Generate data for {} first.",
                    c.name, fk.ref_table, fk.ref_table
                )));
            }
        }
        plans.push(ColumnPlan {
            name: c.name.clone(),
            kind: g.kind,
            max_len: sql_type.max_len(),
            int_range: int_range(&lower, &sql_type),
            decimal: decimal_range(&lower, &sql_type),
            enum_values: if sql_type.base == "enum" || sql_type.base == "set" { sql_type.args.clone() } else { Vec::new() },
            sql_type,
            unique,
            next_seq,
            references,
            seen: HashSet::new(),
        });
    }
    if plans.is_empty() {
        return Err(AppError::validation("Choose a generator for at least one column"));
    }
    Ok(plans)
}

struct Gen {
    rng: fastrand::Rng,
    now: i64,
}

impl Gen {
    fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
        list[self.rng.usize(..list.len())]
    }

    fn lorem(&mut self, words: usize) -> String {
        let mut s = (0..words).map(|_| self.pick(LOREM)).collect::<Vec<_>>().join(" ");
        if let Some(first) = s.get(..1) {
            s.replace_range(..1, &first.to_ascii_uppercase());
        }
        s
    }

    fn sentence(&mut self) -> String {
        let n = self.rng.usize(6..13);
        format!("{}.", self.lorem(n))
    }

    fn timestamp(&mut self) -> chrono::NaiveDateTime {
        // Within the last three years.
        let secs = self.now - self.rng.i64(0..3 * 365 * 86_400);
        chrono::DateTime::from_timestamp(secs, 0).unwrap_or_default().naive_utc()
    }

    fn hex(&mut self, n: usize) -> String {
        (0..n).map(|_| char::from_digit(self.rng.u32(0..16), 16).unwrap_or('0')).collect()
    }

    fn text(&mut self, kind: GeneratorKind, row: u64) -> String {
        use GeneratorKind as K;
        match kind {
            K::FirstName => self.pick(FIRST_NAMES).to_owned(),
            K::LastName => self.pick(LAST_NAMES).to_owned(),
            K::FullName => format!("{} {}", self.pick(FIRST_NAMES), self.pick(LAST_NAMES)),
            K::Email => format!(
                "{}.{}{}@{}",
                self.pick(FIRST_NAMES).to_ascii_lowercase(),
                self.pick(LAST_NAMES).to_ascii_lowercase(),
                self.rng.u32(1..1000),
                self.pick(DOMAINS)
            ),
            K::Username => format!(
                "{}{}{}",
                self.pick(FIRST_NAMES).to_ascii_lowercase(),
                self.pick(LAST_NAMES)[..1].to_ascii_lowercase(),
                self.rng.u32(1..1000)
            ),
            K::Phone => format!("({:03}) {:03}-{:04}", self.rng.u32(201..990), self.rng.u32(200..1000), self.rng.u32(0..10_000)),
            K::Company => format!("{} {}", self.pick(LAST_NAMES), self.pick(COMPANY_SUFFIXES)),
            K::City => self.pick(CITIES).to_owned(),
            K::State => self.pick(STATES).to_owned(),
            K::Country => self.pick(COUNTRIES).to_owned(),
            K::Address => format!("{} {} {}", self.rng.u32(1..9999), self.pick(STREETS), self.pick(STREET_SUFFIXES)),
            K::ZipCode => format!("{:05}", self.rng.u32(1000..99_999)),
            K::Url => format!("https://www.{}{}.{}/{}", self.pick(LOREM), self.pick(LOREM), self.pick(TLDS), self.pick(LOREM)),
            K::Uuid => uuid::Uuid::new_v4().to_string(),
            K::IpAddress => format!("{}.{}.{}.{}", self.rng.u8(1..255), self.rng.u8(0..=255), self.rng.u8(0..=255), self.rng.u8(1..255)),
            K::Word => self.pick(LOREM).to_owned(),
            K::Title => format!("{} {} {}", self.pick(ADJECTIVES), self.pick(MATERIALS), self.pick(NOUNS)),
            K::Sentence => self.sentence(),
            K::Paragraph => (0..self.rng.usize(3..6)).map(|_| self.sentence()).collect::<Vec<_>>().join(" "),
            K::Slug => {
                let n = self.rng.usize(2..5);
                (0..n).map(|_| self.pick(LOREM)).collect::<Vec<_>>().join("-")
            }
            K::Code => {
                let letters: String = (0..3).map(|_| self.rng.uppercase()).collect();
                format!("{letters}-{:04}", self.rng.u32(0..10_000))
            }
            K::Color => format!("#{}", self.hex(6)),
            K::Hash => self.hex(64),
            K::Json => format!(
                r#"{{"id":{},"name":"{}","active":{},"tags":["{}","{}"]}}"#,
                row + 1,
                self.pick(LOREM),
                self.rng.bool(),
                self.pick(LOREM),
                self.pick(LOREM)
            ),
            _ => self.pick(LOREM).to_owned(),
        }
    }

    fn value(&mut self, p: &mut ColumnPlan, row: u64) -> Value {
        use GeneratorKind as K;
        match p.kind {
            K::Skip | K::Null => Value::Null,
            K::Sequence => {
                p.next_seq += 1;
                Value::Int(p.next_seq - 1)
            }
            K::Integer if p.unique => {
                p.next_seq += 1;
                Value::Int(p.next_seq - 1)
            }
            K::Integer => Value::Int(self.rng.i64(p.int_range.0..=p.int_range.1)),
            K::Decimal => {
                let (lo, hi, scale) = p.decimal;
                Value::Decimal(format!("{:.scale$}", lo + self.rng.f64() * (hi - lo)))
            }
            K::Boolean => Value::Int(self.rng.bool() as i64),
            K::Date => Value::Text(self.timestamp().format("%Y-%m-%d").to_string()),
            K::DateTime => Value::Text(self.timestamp().format("%Y-%m-%d %H:%M:%S").to_string()),
            K::Time => Value::Text(format!("{:02}:{:02}:{:02}", self.rng.u32(0..24), self.rng.u32(0..60), self.rng.u32(0..60))),
            K::Year => Value::Int(self.rng.i64(1990..=2025).min(p.sql_type.int_max())),
            K::EnumValue if !p.enum_values.is_empty() => Value::Text(p.enum_values[self.rng.usize(..p.enum_values.len())].clone()),
            K::Bytes => {
                let n = p.max_len.unwrap_or(16).min(16);
                Value::Bytes((0..n).map(|_| self.rng.u8(..)).collect())
            }
            K::ForeignKey if p.references.is_empty() => Value::Null,
            K::ForeignKey => p.references[self.rng.usize(..p.references.len())].clone(),
            K::Json => Value::Json(self.text(K::Json, row)),
            kind => {
                let mut s = self.text(kind, row);
                if p.unique {
                    // Retry, then disambiguate with the row number.
                    for _ in 0..10 {
                        if !p.seen.contains(&truncate(&s, p.max_len)) {
                            break;
                        }
                        s = self.text(kind, row);
                    }
                    if p.seen.contains(&truncate(&s, p.max_len)) {
                        let suffix = format!("-{}", row + 1);
                        let keep = p.max_len.map_or(s.len(), |n| n.saturating_sub(suffix.len()));
                        s = format!("{}{suffix}", truncate(&s, Some(keep)));
                    }
                }
                let s = truncate(&s, p.max_len);
                if p.unique {
                    p.seen.insert(s.clone());
                }
                Value::Text(s)
            }
        }
    }
}

fn truncate(s: &str, max: Option<usize>) -> String {
    match max {
        Some(n) if s.chars().count() > n => s.chars().take(n).collect(),
        _ => s.to_owned(),
    }
}

fn new_gen() -> Gen {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    Gen { rng: fastrand::Rng::new(), now }
}

/// A few generated rows, not inserted, so the user can check the choices.
pub async fn preview_generated(driver: &dyn Driver, db: &str, table: &str, columns: &[ColumnGenerator], rows: usize) -> Result<Page> {
    let info = driver.table_info(db, table).await?;
    let mut plans = plan(driver, db, &info, columns, false, rows as u64).await?;
    let mut g = new_gen();
    let metas = plans
        .iter()
        .map(|p| ColumnMeta {
            name: p.name.clone(),
            type_name: info.columns.iter().find(|c| c.name == p.name).map(|c| c.data_type.clone()).unwrap_or_default(),
            hidden: false,
        })
        .collect();
    let rows = (0..rows as u64).map(|r| plans.iter_mut().map(|p| g.value(p, r).to_cell()).collect()).collect();
    Ok(Page { columns: metas, rows })
}

/// Inserts `spec.rows` generated rows in one transaction; nothing is inserted when a row fails.
pub async fn generate_data(
    driver: &dyn Driver,
    db: &str,
    table: &str,
    spec: &GenerateSpec,
    progress: ProgressFn,
    cancel: CancellationToken,
) -> Result<GenerateReport> {
    if spec.rows == 0 || spec.rows > MAX_GENERATED_ROWS {
        return Err(AppError::validation(format!("Generate between 1 and {MAX_GENERATED_ROWS} rows")));
    }
    if cancel.is_cancelled() {
        return Err(AppError::cancelled());
    }
    let info = driver.table_info(db, table).await?;
    let mut plans = plan(driver, db, &info, &spec.columns, spec.empty_first, spec.rows).await?;
    if cancel.is_cancelled() {
        return Err(AppError::cancelled());
    }
    let names: Vec<String> = plans.iter().map(|p| p.name.clone()).collect();
    let total = spec.rows;
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<ImportBatch>>(4);
    let feed = async move {
        let mut g = new_gen();
        let mut done = 0u64;
        progress(Progress { done, total: Some(total) });
        while done < total {
            if cancel.is_cancelled() {
                let _ = tx.send(Err(AppError::cancelled())).await;
                return;
            }
            let n = (total - done).min(BATCH as u64);
            let batch: ImportBatch = (done..done + n).map(|r| (r + 1, plans.iter_mut().map(|p| g.value(p, r)).collect())).collect();
            if tx.send(Ok(batch)).await.is_err() {
                return;
            }
            done += n;
            progress(Progress { done, total: Some(total) });
        }
        if cancel.is_cancelled() {
            let _ = tx.send(Err(AppError::cancelled())).await;
        }
    };
    let (report, ()) = tokio::join!(
        driver.import_rows(db, table, &names, rx, crate::transfer::ImportOptions { empty_first: spec.empty_first, ..Default::default() }),
        feed
    );
    let report = report?;
    if let Some(f) = report.failed.first() {
        return Err(AppError::sql(format!("Row {} could not be inserted: {}", f.line, f.message)));
    }
    Ok(GenerateReport { rows: report.inserted })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_types_and_enum_values() {
        let t = SqlType::parse("enum('a','it''s','b,c')");
        assert_eq!(t.base, "enum");
        assert_eq!(t.args, ["a", "it's", "b,c"]);
        let d = SqlType::parse("decimal(5,2) unsigned");
        assert!(d.unsigned);
        assert_eq!(d.decimal_bounds(), (999.0, 2));
        assert_eq!(SqlType::parse("tinyint unsigned").int_max(), 255);
        assert_eq!(SqlType::parse("VARCHAR(40)").max_len(), Some(40));
    }

    #[test]
    fn ranges_stay_within_the_type() {
        assert_eq!(int_range("age", &SqlType::parse("tinyint")), (18, 90));
        assert_eq!(int_range("qty", &SqlType::parse("bit(1)")), (0, 1));
        assert_eq!(decimal_range("price", &SqlType::parse("decimal(4,2)")), (1.0, 99.0, 2));
    }

    #[test]
    fn truncates_to_the_column_length_and_keeps_unique_values_distinct() {
        let mut g = new_gen();
        let mut p = ColumnPlan {
            name: "code".into(),
            kind: GeneratorKind::Word,
            sql_type: SqlType::parse("varchar(6)"),
            max_len: Some(6),
            int_range: (1, 10),
            decimal: (0.0, 1.0, 2),
            enum_values: Vec::new(),
            unique: true,
            next_seq: 1,
            references: Vec::new(),
            seen: HashSet::new(),
        };
        let values: Vec<String> = (0..200)
            .map(|r| match g.value(&mut p, r) {
                Value::Text(s) => s,
                other => panic!("{other:?}"),
            })
            .collect();
        assert!(values.iter().all(|v| v.chars().count() <= 6));
        assert_eq!(values.iter().collect::<HashSet<_>>().len(), 200);
    }
}
