//! A small SQL tokenizer for reading and rewriting DDL text. Whitespace and comments are skipped and string literals
//! stay opaque, so a name is only recognized where it is a name.

use crate::Engine;
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TokenKind {
    /// A bare word: a keyword, an unquoted name or a number.
    Word,
    /// A quoted name (`` `x` ``, `"x"` on SQLite, `[x]`), unescaped.
    Quoted(String),
    /// A string literal.
    Text,
    /// Any other character.
    Symbol(char),
}

/// One token and its byte range in the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: TokenKind,
}

impl Token {
    /// The name a word or quoted name spells.
    pub fn name<'a>(&'a self, sql: &'a str) -> Option<Cow<'a, str>> {
        match &self.kind {
            TokenKind::Word => Some(Cow::Borrowed(&sql[self.start..self.end])),
            TokenKind::Quoted(name) => Some(Cow::Borrowed(name)),
            TokenKind::Text | TokenKind::Symbol(_) => None,
        }
    }

    /// A bare word equal to `word`, ignoring case.
    pub fn is_keyword(&self, sql: &str, word: &str) -> bool {
        self.kind == TokenKind::Word && sql[self.start..self.end].eq_ignore_ascii_case(word)
    }

    pub fn is_symbol(&self, c: char) -> bool {
        self.kind == TokenKind::Symbol(c)
    }
}

/// Tokens of `sql` in the dialect of `engine` (`#` comments, backslash escapes and `"` strings on MySQL/MariaDB).
pub(crate) struct Tokens<'a> {
    sql: &'a str,
    pos: usize,
    mysql: bool,
}

impl<'a> Tokens<'a> {
    pub fn new(sql: &'a str, engine: Engine) -> Self {
        Self { sql, pos: 0, mysql: engine != Engine::Sqlite }
    }

    fn skip_trivia(&mut self) {
        let bytes = self.sql.as_bytes();
        loop {
            while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            let rest = &bytes[self.pos..];
            if rest.starts_with(b"--") || (self.mysql && rest.starts_with(b"#")) {
                self.pos = rest.iter().position(|b| *b == b'\n').map_or(bytes.len(), |n| self.pos + n + 1);
            } else if rest.starts_with(b"/*") {
                // MySQL's executable comments (`/*!40101 ... */`) are skipped too: callers only look for names.
                self.pos = self.sql[self.pos + 2..].find("*/").map_or(bytes.len(), |n| self.pos + 2 + n + 2);
            } else {
                return;
            }
        }
    }

    /// End of the quoted section opened at `start` by `open`; a doubled closing quote (or, in MySQL strings, a
    /// backslash) escapes it.
    fn quoted_end(&self, start: usize, close: u8, backslash: bool) -> usize {
        let bytes = self.sql.as_bytes();
        let mut i = start + 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' if backslash => i += 2,
                c if c == close && close != b']' && bytes.get(i + 1) == Some(&close) => i += 2,
                c if c == close => return i + 1,
                _ => i += 1,
            }
        }
        bytes.len()
    }
}

impl Iterator for Tokens<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        self.skip_trivia();
        let bytes = self.sql.as_bytes();
        let start = self.pos;
        let first = *bytes.get(start)?;
        let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80;
        let (end, kind) = match first {
            b'\'' => (self.quoted_end(start, b'\'', self.mysql), TokenKind::Text),
            b'"' if self.mysql => (self.quoted_end(start, b'"', true), TokenKind::Text),
            b'`' | b'"' | b'[' => {
                let close = if first == b'[' { b']' } else { first };
                let end = self.quoted_end(start, close, false);
                let inner_end = if end > start + 1 && bytes[end - 1] == close { end - 1 } else { end };
                let inner = &self.sql[start + 1..inner_end];
                let name = match close {
                    b']' => inner.to_owned(),
                    _ => inner.replace(&format!("{0}{0}", close as char), &(close as char).to_string()),
                };
                (end, TokenKind::Quoted(name))
            }
            b if word(b) => (start + bytes[start..].iter().position(|b| !word(*b)).unwrap_or(bytes.len() - start), TokenKind::Word),
            _ => {
                let c = self.sql[start..].chars().next()?;
                (start + c.len_utf8(), TokenKind::Symbol(c))
            }
        };
        self.pos = end;
        Some(Token { start, end, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sql: &str, engine: Engine) -> Vec<String> {
        Tokens::new(sql, engine).filter_map(|t| t.name(sql).map(Cow::into_owned)).collect()
    }

    #[test]
    fn skips_comments_and_strings_and_unescapes_quoted_names() {
        let sql = "CREATE /*!32312 IF NOT EXISTS*/ `a``b` -- note\n # hash\n 'it''s' \"x\"";
        assert_eq!(names(sql, Engine::MySql), ["CREATE", "a`b"]);
        assert_eq!(names("REFERENCES \"pa\"\"rent\" ([id]) 'REFERENCES x'", Engine::Sqlite), ["REFERENCES", "pa\"rent", "id"]);
    }

    #[test]
    fn mysql_strings_honor_backslash_escapes() {
        let sql = r"DEFAULT 'a\'b' NEXT";
        assert_eq!(names(sql, Engine::MySql), ["DEFAULT", "NEXT"]);
    }
}
