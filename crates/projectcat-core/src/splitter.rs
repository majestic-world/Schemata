use crate::Engine;

/// Splits a SQL script into statements, honoring quotes, comments, the MySQL `DELIMITER`
/// directive and SQLite trigger bodies (`BEGIN ... END`).
pub fn split_statements(sql: &str, engine: Engine) -> Vec<String> {
    let mysql = engine != Engine::Sqlite;
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut has_code = false;
    let mut delimiter = String::from(";");
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut trigger_depth: i32 = 0;
    let mut i = 0;

    // Records a completed bare word (keyword or identifier) for trigger tracking.
    let finish_word = |word: &mut String, words: &mut Vec<String>, depth: &mut i32| {
        if word.is_empty() {
            return;
        }
        let upper = word.to_ascii_uppercase();
        let is_trigger = is_sqlite_trigger(words);
        words.push(upper.clone());
        word.clear();
        if !mysql && (is_trigger || is_sqlite_trigger(words)) {
            match upper.as_str() {
                "BEGIN" | "CASE" => *depth += 1,
                "END" => *depth -= 1,
                _ => {}
            }
        }
    };

    while i < chars.len() {
        // DELIMITER directive: only at the start of a statement, consumes the whole line.
        if mysql && !has_code && at_line_start(&chars, i) {
            let line_end = chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
            let line: String = chars[i..line_end].iter().collect();
            let trimmed = line.trim();
            if trimmed.len() > 10 && trimmed[..10].eq_ignore_ascii_case("DELIMITER ") {
                delimiter = trimmed[10..].trim().to_owned();
                buf.clear();
                i = line_end;
                continue;
            }
        }
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        // Comments are kept in the statement text but do not count as code.
        let line_comment = (c == '-' && next == Some('-') && (!mysql || chars.get(i + 2).is_none_or(|c| c.is_whitespace())))
            || (mysql && c == '#');
        if line_comment {
            finish_word(&mut word, &mut words, &mut trigger_depth);
            while i < chars.len() && chars[i] != '\n' {
                buf.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            finish_word(&mut word, &mut words, &mut trigger_depth);
            // MySQL executable comments (/*! ... */) are code.
            if mysql && chars.get(i + 2) == Some(&'!') {
                has_code = true;
            }
            let end = find_seq(&chars, i + 2, &['*', '/']).map_or(chars.len(), |p| p + 2);
            buf.extend(&chars[i..end]);
            i = end;
            continue;
        }

        // Statement terminator.
        if starts_with(&chars, i, &delimiter) && trigger_depth <= 0 {
            finish_word(&mut word, &mut words, &mut trigger_depth);
            if trigger_depth <= 0 {
                push_statement(&mut out, &mut buf, &mut has_code);
                words.clear();
                trigger_depth = 0;
                i += delimiter.chars().count();
                continue;
            }
        }

        // Quoted strings and identifiers.
        let close = match c {
            '\'' | '"' | '`' => Some(c),
            '[' if !mysql => Some(']'),
            _ => None,
        };
        if let Some(close) = close {
            finish_word(&mut word, &mut words, &mut trigger_depth);
            has_code = true;
            buf.push(c);
            i += 1;
            while i < chars.len() {
                let ch = chars[i];
                buf.push(ch);
                i += 1;
                if mysql && ch == '\\' && close != '`' && i < chars.len() {
                    buf.push(chars[i]);
                    i += 1;
                } else if ch == close {
                    if chars.get(i) == Some(&close) && close != ']' {
                        buf.push(close);
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            words.push(String::from("\u{0}"));
            continue;
        }

        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            finish_word(&mut word, &mut words, &mut trigger_depth);
        }
        if !c.is_whitespace() {
            has_code = true;
        }
        buf.push(c);
        i += 1;
    }
    push_statement(&mut out, &mut buf, &mut has_code);
    out
}

fn is_sqlite_trigger(words: &[String]) -> bool {
    match words {
        [a, b, ..] if a == "CREATE" && b == "TRIGGER" => true,
        [a, b, c, ..] if a == "CREATE" && (b == "TEMP" || b == "TEMPORARY") && c == "TRIGGER" => true,
        _ => false,
    }
}

fn push_statement(out: &mut Vec<String>, buf: &mut String, has_code: &mut bool) {
    if *has_code {
        out.push(buf.trim().to_owned());
    }
    buf.clear();
    *has_code = false;
}

fn at_line_start(chars: &[char], i: usize) -> bool {
    chars[..i].iter().rev().take_while(|&&c| c != '\n').all(|c| c.is_whitespace())
}

fn starts_with(chars: &[char], i: usize, pat: &str) -> bool {
    let mut j = i;
    for p in pat.chars() {
        if chars.get(j).is_none_or(|c| !c.eq_ignore_ascii_case(&p)) {
            return false;
        }
        j += 1;
    }
    true
}

fn find_seq(chars: &[char], from: usize, seq: &[char]) -> Option<usize> {
    (from..chars.len().saturating_sub(seq.len() - 1)).find(|&p| chars[p..p + seq.len()] == *seq)
}
