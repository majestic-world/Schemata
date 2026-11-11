use crate::Engine;

/// Splits a SQL script into statements, honoring quotes, comments, the MySQL `DELIMITER`
/// directive and SQLite trigger bodies (`BEGIN ... END`).
pub fn split_statements(sql: &str, engine: Engine) -> Vec<String> {
    let mut splitter = Splitter::new(engine);
    let mut out = splitter.feed(sql);
    out.extend(splitter.finish());
    out
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Normal,
    BlockComment,
    Quote(char),
}

/// Incremental statement splitter: feed a script in whole lines, collect statements as they complete.
pub struct Splitter {
    mysql: bool,
    delimiter: String,
    state: State,
    buf: String,
    has_code: bool,
    word: String,
    /// Leading words of the current statement (enough to recognize `CREATE [TEMP] TRIGGER`).
    words: Vec<String>,
    trigger_depth: i32,
    out: Vec<String>,
}

impl Splitter {
    pub fn new(engine: Engine) -> Self {
        Splitter {
            mysql: engine != Engine::Sqlite,
            delimiter: ";".into(),
            state: State::Normal,
            buf: String::new(),
            has_code: false,
            word: String::new(),
            words: Vec::new(),
            trigger_depth: 0,
            out: Vec::new(),
        }
    }

    /// Consumes text and returns the statements completed so far. Chunks must end at line boundaries
    /// (except the last one), so that line-based syntax such as `DELIMITER` is recognized.
    pub fn feed(&mut self, text: &str) -> Vec<String> {
        for line in text.split_inclusive('\n') {
            self.line(line);
        }
        std::mem::take(&mut self.out)
    }

    /// Returns the trailing statement, if the script did not end with a delimiter.
    pub fn finish(mut self) -> Vec<String> {
        self.finish_word();
        self.push_statement();
        self.out
    }

    fn line(&mut self, line: &str) {
        let trimmed = line.trim();
        if self.mysql
            && self.state == State::Normal
            && !self.has_code
            && trimmed.len() > 10
            && trimmed.get(..10).is_some_and(|prefix| prefix.eq_ignore_ascii_case("DELIMITER "))
        {
            self.delimiter = trimmed[10..].trim().to_owned();
            self.buf.clear();
            return;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            match self.state {
                State::BlockComment => {
                    self.buf.push(c);
                    if c == '*' && next == Some('/') {
                        self.buf.push('/');
                        self.state = State::Normal;
                        i += 1;
                    }
                }
                State::Quote(close) => {
                    self.buf.push(c);
                    if self.mysql && c == '\\' && close != '`' {
                        if let Some(n) = next {
                            self.buf.push(n);
                            i += 1;
                        }
                    } else if c == close {
                        if next == Some(close) && close != ']' {
                            self.buf.push(close);
                            i += 1;
                        } else {
                            self.state = State::Normal;
                        }
                    }
                }
                State::Normal => {
                    let line_comment =
                        (c == '-' && next == Some('-') && (!self.mysql || chars.get(i + 2).is_none_or(|c| c.is_whitespace())))
                            || (self.mysql && c == '#');
                    if line_comment {
                        // Comments stay in the statement text but do not count as code.
                        self.finish_word();
                        self.buf.extend(&chars[i..]);
                        return;
                    }
                    if c == '/' && next == Some('*') {
                        self.finish_word();
                        // MySQL executable comments (/*! ... */) are code.
                        if self.mysql && chars.get(i + 2) == Some(&'!') {
                            self.has_code = true;
                        }
                        self.buf.push_str("/*");
                        self.state = State::BlockComment;
                        i += 2;
                        continue;
                    }
                    if starts_with(&chars, i, &self.delimiter) {
                        // The pending word (e.g. a trigger's END) must be counted first.
                        self.finish_word();
                        if self.trigger_depth <= 0 {
                            self.push_statement();
                            i += self.delimiter.chars().count();
                            continue;
                        }
                    }
                    let close = match c {
                        '\'' | '"' | '`' => Some(c),
                        '[' if !self.mysql => Some(']'),
                        _ => None,
                    };
                    if let Some(close) = close {
                        self.finish_word();
                        self.has_code = true;
                        self.buf.push(c);
                        self.state = State::Quote(close);
                        self.words.push("\u{0}".into());
                        i += 1;
                        continue;
                    }
                    if c.is_alphanumeric() || c == '_' {
                        self.word.push(c);
                    } else {
                        self.finish_word();
                    }
                    if !c.is_whitespace() {
                        self.has_code = true;
                    }
                    self.buf.push(c);
                }
            }
            i += 1;
        }
    }

    /// Records a completed bare word (keyword or identifier) for SQLite trigger tracking.
    fn finish_word(&mut self) {
        if self.word.is_empty() {
            return;
        }
        let upper = self.word.to_ascii_uppercase();
        self.word.clear();
        if self.words.len() < 4 {
            self.words.push(upper.clone());
        }
        if !self.mysql && is_sqlite_trigger(&self.words) {
            match upper.as_str() {
                "BEGIN" | "CASE" => self.trigger_depth += 1,
                "END" => self.trigger_depth -= 1,
                _ => {}
            }
        }
    }

    fn push_statement(&mut self) {
        if self.has_code {
            self.out.push(self.buf.trim().to_owned());
        }
        self.buf.clear();
        self.has_code = false;
        self.words.clear();
        self.trigger_depth = 0;
    }
}

fn is_sqlite_trigger(words: &[String]) -> bool {
    match words {
        [a, b, ..] if a == "CREATE" && b == "TRIGGER" => true,
        [a, b, c, ..] if a == "CREATE" && (b == "TEMP" || b == "TEMPORARY") && c == "TRIGGER" => true,
        _ => false,
    }
}

fn starts_with(chars: &[char], i: usize, pat: &str) -> bool {
    pat.chars().enumerate().all(|(k, p)| chars.get(i + k).is_some_and(|c| c.eq_ignore_ascii_case(&p)))
}
