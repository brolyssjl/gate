//! Minimal, dependency-free JSON value + parser for the conformance suite.
//!
//! `rust/tests/` keeps the crate's zero-dependency constraint (std only, no
//! `serde_json`), and integration tests compile as separate crates with no
//! access to the binary's internal `core::json` module (the crate exposes
//! only a `[[bin]]`, no `[lib]`), so this is a small, self-contained copy
//! covering just what the conformance assertions need: parse, `.get(key)`,
//! and the scalar/array accessors.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    /// `Object.keys(value).sort()` - the shape most conformance schema
    /// assertions check.
    pub fn sorted_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .as_object()
            .unwrap_or(&[])
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }

    /// Convenience for `value.get(key).and_then(Value::as_str)`.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    /// Convenience for `value.get(key).and_then(Value::as_bool)`.
    pub fn bool_at(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ParseError {}

struct Parser<'a> {
    chars: Vec<char>,
    pos: usize,
    src: &'a str,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Parser {
            chars: src.chars().collect(),
            pos: 0,
            src,
        }
    }

    fn err(&self, msg: &str) -> ParseError {
        ParseError(format!("{msg} at position {} in {:?}", self.pos, self.src))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), ParseError> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{c}'")))
        }
    }

    fn parse_value(&mut self) -> Result<Value, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Value::String(self.parse_string()?)),
            Some('t') => self.parse_literal("true", Value::Bool(true)),
            Some('f') => self.parse_literal("false", Value::Bool(false)),
            Some('n') => self.parse_literal("null", Value::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            _ => Err(self.err("unexpected end of input or invalid token")),
        }
    }

    fn parse_literal(&mut self, lit: &str, value: Value) -> Result<Value, ParseError> {
        for expect in lit.chars() {
            if self.bump() != Some(expect) {
                return Err(self.err(&format!("expected literal '{lit}'")));
            }
        }
        Ok(value)
    }

    fn parse_object(&mut self) -> Result<Value, ParseError> {
        self.expect('{')?;
        let mut entries: Vec<(String, Value)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some('"') {
                return Err(self.err("expected a string key"));
            }
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                entries.push((key, value));
            }
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some('}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected ',' or '}'")),
            }
        }
        Ok(Value::Object(entries))
    }

    fn parse_array(&mut self) -> Result<Value, ParseError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.pos += 1;
                }
                Some(']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.err("expected ',' or ']'")),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let c = self.bump().ok_or_else(|| self.err("unterminated string"))?;
            match c {
                '"' => break,
                '\\' => {
                    let esc = self.bump().ok_or_else(|| self.err("unterminated escape"))?;
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let cp = self.parse_hex4()?;
                            if let Some(ch) = char::from_u32(cp) {
                                out.push(ch);
                            } else {
                                out.push('\u{FFFD}');
                            }
                        }
                        other => return Err(self.err(&format!("invalid escape '\\{other}'"))),
                    }
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }

    fn parse_hex4(&mut self) -> Result<u32, ParseError> {
        let mut cp = 0u32;
        for _ in 0..4 {
            let c = self
                .bump()
                .ok_or_else(|| self.err("truncated \\u escape"))?;
            let digit = c
                .to_digit(16)
                .ok_or_else(|| self.err("invalid hex digit in \\u escape"))?;
            cp = cp * 16 + digit;
        }
        Ok(cp)
    }

    fn parse_number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        let mut is_float = false;
        if self.peek() == Some('.') {
            is_float = true;
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if text.is_empty() || text == "-" {
            return Err(self.err("invalid number"));
        }
        if is_float {
            text.parse::<f64>()
                .map(Value::Float)
                .map_err(|_| self.err("invalid number"))
        } else {
            match text.parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => text
                    .parse::<f64>()
                    .map(Value::Float)
                    .map_err(|_| self.err("invalid number")),
            }
        }
    }
}

/// Parse a JSON document, accepting what `JSON.parse` accepts.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    let mut parser = Parser::new(text);
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.chars.len() {
        return Err(parser.err("unexpected trailing content"));
    }
    Ok(value)
}
