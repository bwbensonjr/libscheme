//! The s-expression reader — the Rust analogue of `scheme_read.c`.
//!
//! Behavior is mirrored faithfully from the C `scheme_read`, including its
//! quirks: string escapes are *literal* (a backslash just protects the next
//! character, so `"\n"` reads as the two-character... no — as the single
//! character `n`, exactly as the C reader does), symbols are case-folded by the
//! interner, and `#x`/`#b`/`#o` radix literals, named characters, and `#| |#`
//! block comments are all supported.
//!
//! In C the reader pulls characters from a port via `scheme_getc`/`scheme_ungetc`
//! with one- and two-character lookahead. Here a [`Reader`] wraps the input as a
//! `Vec<char>` with a cursor, giving O(1) `getc`/`peek`/`peek2`/`ungetc`. Wiring
//! the reader to live ports lands in Phase 5; for now it reads from a string.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::Value;

pub struct Reader {
    chars: Vec<char>,
    pos: usize,
}

impl Reader {
    pub fn new(input: &str) -> Self {
        Reader {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    /// Build a reader over an explicit char buffer, starting at `start`.
    pub fn from_chars(chars: Vec<char>, start: usize) -> Self {
        Reader { chars, pos: start }
    }

    /// Current cursor position — used to advance a port after reading one datum.
    pub fn pos(&self) -> usize {
        self.pos
    }

    fn getc(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn ungetc(&mut self) {
        debug_assert!(self.pos > 0);
        self.pos -= 1;
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    /// Read every form until EOF — convenient for loading files and tests.
    pub fn read_all(&mut self, it: &mut Interp) -> SchemeResult<Vec<Value>> {
        let mut forms = Vec::new();
        loop {
            match self.read(it)? {
                Value::Eof => break,
                v => forms.push(v),
            }
        }
        Ok(forms)
    }

    /// Read one datum. Returns [`Value::Eof`] at end of input, like the C
    /// `scheme_read` returning `scheme_eof`.
    pub fn read(&mut self, it: &mut Interp) -> SchemeResult {
        loop {
            let ch = match self.getc() {
                None => return Ok(Value::Eof),
                Some(c) => c,
            };
            if ch.is_whitespace() {
                continue;
            }
            match ch {
                ')' => return Err(SchemeError::msg("read: unexpected ')'")),
                '(' | '[' => return self.read_list(it, matching_close(ch)),
                '"' => return self.read_string(),
                '\'' => return self.read_wrapped(it, "quote"),
                '`' => return self.read_wrapped(it, "quasiquote"),
                ',' => {
                    if self.peek() == Some('@') {
                        self.getc();
                        return self.read_wrapped(it, "unquote-splicing");
                    }
                    return self.read_wrapped(it, "unquote");
                }
                ';' => {
                    // line comment: discard through newline (or EOF)
                    loop {
                        match self.getc() {
                            None => return Ok(Value::Eof),
                            Some('\n') => break,
                            Some(_) => {}
                        }
                    }
                    continue;
                }
                '+' | '-' => {
                    if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        self.ungetc();
                        return self.read_number(it);
                    }
                    self.ungetc();
                    return self.read_symbol(it);
                }
                '#' => match self.getc() {
                    Some('(') => return self.read_vector(it),
                    Some('\\') => return self.read_character(),
                    Some('t') | Some('T') => return Ok(Value::Bool(true)),
                    Some('f') | Some('F') => return Ok(Value::Bool(false)),
                    Some('x') | Some('X') => return Ok(self.read_radix(16)),
                    Some('b') | Some('B') => return Ok(self.read_radix(2)),
                    Some('o') | Some('O') => return Ok(self.read_radix(8)),
                    Some('|') => {
                        self.skip_block_comment()?;
                        continue;
                    }
                    _ => return Err(SchemeError::msg("read: unexpected `#'")),
                },
                c if c.is_ascii_digit() => {
                    self.ungetc();
                    return self.read_number(it);
                }
                _ => {
                    self.ungetc();
                    return self.read_symbol(it);
                }
            }
        }
    }

    /// `(` (or `[`) has been consumed; read up to the matching close, honoring
    /// dotted-pair notation `(a . b)`.
    fn read_list(&mut self, it: &mut Interp, close: char) -> SchemeResult {
        let mut items: Vec<Value> = Vec::new();
        let mut tail = Value::Null;
        loop {
            self.skip_atmosphere()?;
            match self.peek() {
                None => return Err(SchemeError::msg("read: end of file in list")),
                Some(c) if c == close => {
                    self.getc();
                    break;
                }
                Some('.') if self.peek2().is_none_or(|c| c.is_whitespace()) => {
                    self.getc(); // consume '.'
                    tail = self.read(it)?;
                    self.skip_atmosphere()?;
                    if self.peek() != Some(close) {
                        return Err(SchemeError::msg("read: malformed list"));
                    }
                    self.getc();
                    break;
                }
                Some(_) => items.push(self.read(it)?),
            }
        }
        // Build the list with the (possibly non-null) tail.
        let mut acc = tail;
        for v in items.into_iter().rev() {
            acc = Value::cons(v, acc);
        }
        Ok(acc)
    }

    /// `"` has been consumed. Matches C: a backslash protects the next char
    /// literally; there is no `\n`-to-newline translation.
    fn read_string(&mut self) -> SchemeResult {
        let mut s = String::new();
        loop {
            match self.getc() {
                None => return Err(SchemeError::msg("read: end of file in string")),
                Some('"') => break,
                Some('\\') => match self.getc() {
                    None => return Err(SchemeError::msg("read: end of file in string")),
                    Some(c) => s.push(c),
                },
                Some(c) => s.push(c),
            }
        }
        Ok(Value::make_string(s))
    }

    /// Wrap the next datum as `(sym datum)` — quote/quasiquote/unquote readers.
    fn read_wrapped(&mut self, it: &mut Interp, sym: &str) -> SchemeResult {
        let inner = self.read(it)?;
        let s = it.intern(sym);
        Ok(Value::list(&[Value::Symbol(s), inner]))
    }

    /// `#(` has been consumed; read elements until `)`.
    fn read_vector(&mut self, it: &mut Interp) -> SchemeResult {
        let list = self.read_list(it, ')')?;
        // Flatten the proper list into a Vec.
        let mut items = Vec::new();
        let mut cur = list;
        while let Value::Pair(p) = &cur {
            let (car, cdr) = {
                let pair = p.borrow();
                (pair.car.clone(), pair.cdr.clone())
            };
            items.push(car);
            cur = cdr;
        }
        Ok(Value::make_vector(items))
    }

    fn read_number(&mut self, it: &mut Interp) -> SchemeResult {
        let mut buf = String::new();
        let mut is_float = false;
        // optional sign
        if let Some(c @ ('+' | '-')) = self.peek() {
            buf.push(c);
            self.getc();
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                buf.push(c);
                self.getc();
            } else if c == '.' || c == 'e' || c == 'E' {
                is_float = true;
                buf.push(c);
                self.getc();
            } else {
                break;
            }
        }
        if is_float {
            match buf.parse::<f64>() {
                Ok(d) => Ok(Value::Double(d)),
                Err(_) => Ok(Value::Symbol(it.intern(&buf))),
            }
        } else {
            match buf.parse::<i64>() {
                Ok(n) => Ok(Value::Int(n)),
                Err(_) => Ok(Value::Symbol(it.intern(&buf))),
            }
        }
    }

    /// Read a radix literal (`#x`/`#b`/`#o` already consumed).
    fn read_radix(&mut self, radix: u32) -> Value {
        let mut n: i64 = 0;
        while let Some(c) = self.peek() {
            match c.to_digit(radix) {
                Some(d) => {
                    n = n * radix as i64 + d as i64;
                    self.getc();
                }
                None => break,
            }
        }
        Value::Int(n)
    }

    fn read_symbol(&mut self, it: &mut Interp) -> SchemeResult {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || matches!(c, '(' | ')' | '[' | ']' | '"' | ';') {
                break;
            }
            s.push(c);
            self.getc();
        }
        Ok(Value::Symbol(it.intern(&s)))
    }

    /// `#\` has been consumed. Reads a named character (`newline`, `space`, …)
    /// or a single literal character.
    fn read_character(&mut self) -> SchemeResult {
        let first = match self.getc() {
            None => return Err(SchemeError::msg("read: end of file after #\\")),
            Some(c) => c,
        };
        // A multi-letter name only when the first char is alphabetic AND the
        // next char is also alphabetic (otherwise `#\s` is just 's').
        if first.is_ascii_alphabetic() && self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            let mut name = String::new();
            name.push(first);
            while let Some(c) = self.peek() {
                if c.is_ascii_alphabetic() {
                    name.push(c);
                    self.getc();
                } else {
                    break;
                }
            }
            match named_char(&name) {
                Some(c) => Ok(Value::Char(c)),
                None => Err(SchemeError::msg("read: bad character constant")),
            }
        } else {
            Ok(Value::Char(first))
        }
    }

    /// Skip whitespace and `;` line comments (the `skip_whitespace_comments`
    /// helper); used between list elements.
    fn skip_atmosphere(&mut self) -> SchemeResult<()> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.getc();
                }
                Some(';') => {
                    self.getc();
                    while let Some(c) = self.getc() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// `#|` has been consumed; skip to the matching `|#` (no nesting, as in C).
    fn skip_block_comment(&mut self) -> SchemeResult<()> {
        loop {
            match self.getc() {
                None => return Err(SchemeError::msg("read: end of file in #| comment")),
                Some('|') if self.peek() == Some('#') => {
                    self.getc();
                    return Ok(());
                }
                Some(_) => {}
            }
        }
    }
}

fn matching_close(open: char) -> char {
    match open {
        '[' => ']',
        _ => ')',
    }
}

/// Map a character name to its value (`scheme_read.c`'s `read_character`).
fn named_char(name: &str) -> Option<char> {
    match name.to_ascii_lowercase().as_str() {
        "newline" | "linefeed" => Some('\n'),
        "space" => Some(' '),
        "tab" => Some('\t'),
        "return" => Some('\r'),
        "page" => Some('\u{c}'),
        "backspace" => Some('\u{8}'),
        "rubout" | "delete" => Some('\u{7f}'),
        "nul" | "null" => Some('\0'),
        "escape" | "altmode" => Some('\u{1b}'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_one(src: &str) -> (Interp, Value) {
        let mut it = Interp::new();
        let mut r = Reader::new(src);
        let v = r.read(&mut it).expect("read");
        (it, v)
    }

    #[test]
    fn reads_scalars() {
        assert!(read_one("42").1.eq(&Value::Int(42)));
        assert!(read_one("-7").1.eq(&Value::Int(-7)));
        assert!(read_one("#t").1.eq(&Value::Bool(true)));
        assert!(read_one("#f").1.eq(&Value::Bool(false)));
        assert!(read_one("#\\a").1.eq(&Value::Char('a')));
        assert!(read_one("#\\newline").1.eq(&Value::Char('\n')));
        assert!(read_one("#\\space").1.eq(&Value::Char(' ')));
        match read_one("2.5").1 {
            Value::Double(d) => assert!((d - 2.5).abs() < 1e-9),
            _ => panic!("expected double"),
        }
    }

    #[test]
    fn reads_radix_literals() {
        assert!(read_one("#xff").1.eq(&Value::Int(255)));
        assert!(read_one("#b1010").1.eq(&Value::Int(10)));
        assert!(read_one("#o17").1.eq(&Value::Int(15)));
    }

    #[test]
    fn reads_string_with_literal_escapes() {
        // C semantics: backslash protects the next char literally.
        match &read_one(r#""a\"b\\c""#).1 {
            Value::Str(s) => assert_eq!(*s.borrow(), "a\"b\\c"),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn reads_proper_and_dotted_lists() {
        let (_it, v) = read_one("(1 2 3)");
        let expected = Value::list(&[Value::Int(1), Value::Int(2), Value::Int(3)]);
        assert!(v.equal(&expected));

        let (_it, v) = read_one("(1 . 2)");
        let expected = Value::cons(Value::Int(1), Value::Int(2));
        assert!(v.equal(&expected));
    }

    #[test]
    fn reads_quote_forms() {
        let mut it = Interp::new();
        let mut r = Reader::new("'x");
        let v = r.read(&mut it).unwrap();
        let q = it.intern("quote");
        let x = it.intern("x");
        assert!(v.equal(&Value::list(&[Value::Symbol(q), Value::Symbol(x)])));
    }

    #[test]
    fn reads_vector() {
        let (_it, v) = read_one("#(1 2 3)");
        match &v {
            Value::Vector(ve) => {
                let ve = ve.borrow();
                assert_eq!(ve.len(), 3);
                assert!(ve[0].eq(&Value::Int(1)) && ve[2].eq(&Value::Int(3)));
            }
            _ => panic!("expected vector"),
        }
    }

    #[test]
    fn skips_comments() {
        let mut it = Interp::new();
        let mut r = Reader::new("; a comment\n  #| block |#  99");
        assert!(r.read(&mut it).unwrap().eq(&Value::Int(99)));
    }

    #[test]
    fn symbols_are_case_folded() {
        let mut it = Interp::new();
        let mut r = Reader::new("Foo foo");
        let a = r.read(&mut it).unwrap();
        let b = r.read(&mut it).unwrap();
        assert!(a.eq(&b));
    }

    #[test]
    fn eof_at_end() {
        let mut it = Interp::new();
        let mut r = Reader::new("1");
        assert!(r.read(&mut it).unwrap().eq(&Value::Int(1)));
        assert!(matches!(r.read(&mut it).unwrap(), Value::Eof));
    }
}
