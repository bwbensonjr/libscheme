//! The printer — the Rust analogue of `scheme_print.c`.
//!
//! Two modes mirror the C `escaped` flag: `write` (re-readable: strings quoted
//! and escaped, characters as `#\name`) and `display` (human-facing: raw string
//! and character contents). Both need the [`Interp`] to resolve interned symbol
//! names, so they take `&Interp` — the analogue of the symbol's `string_val`.
//!
//! Fidelity notes carried over from C: doubles print with `%f` (six decimal
//! places), opaque objects print as `#<typename>`, and — as in C — there is no
//! cycle detection, so a `set-cdr!` loop will recurse forever exactly as the
//! original does. Lists use the dotted-tail shorthand for improper lists.

use crate::interp::Interp;
use crate::value::Value;
use std::fmt::Write;

/// `write` — produce the re-readable external representation.
pub fn write_to_string(it: &Interp, obj: &Value) -> String {
    let mut s = String::new();
    print(it, &mut s, obj, true);
    s
}

/// `display` — produce the human-facing representation.
pub fn display_to_string(it: &Interp, obj: &Value) -> String {
    let mut s = String::new();
    print(it, &mut s, obj, false);
    s
}

fn print(it: &Interp, out: &mut String, obj: &Value, escaped: bool) {
    match obj {
        Value::Null => out.push_str("()"),
        Value::Bool(true) => out.push_str("#t"),
        Value::Bool(false) => out.push_str("#f"),
        Value::Eof => out.push_str("#<eof>"),
        Value::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Value::Double(d) => {
            // Match C's `%f` (six decimals, e.g. 3.140000).
            let _ = write!(out, "{d:.6}");
        }
        Value::Char(c) => print_char(out, *c, escaped),
        Value::Symbol(s) => out.push_str(it.resolve(*s)),
        Value::Str(s) => print_string(out, &s.borrow(), escaped),
        Value::Pair(_) => print_pair(it, out, obj, escaped),
        Value::Vector(v) => {
            out.push_str("#(");
            let v = v.borrow();
            for (i, el) in v.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                print(it, out, el, escaped);
            }
            out.push(')');
        }
        // Opaque objects print as #<typename>, like C's `#<type>` fallback.
        Value::Closure(_) | Value::Macro(_) => out.push_str("#<closure>"),
        Value::Prim(p) => {
            let _ = write!(out, "#<primitive:{}>", it.resolve(p.name));
        }
        Value::Continuation(_) => out.push_str("#<continuation>"),
        Value::Syntax(s) => {
            let _ = write!(out, "#<syntax:{}>", it.resolve(s.name));
        }
        Value::Promise(_) => out.push_str("#<promise>"),
        Value::InputPort(_) => out.push_str("#<input-port>"),
        Value::OutputPort(_) => out.push_str("#<output-port>"),
        Value::TypeObject(t) => {
            let _ = write!(out, "#<type:{}>", t.name);
        }
        Value::Struct(s) => {
            let _ = write!(out, "#<{}>", s.borrow().ty.name);
        }
        Value::StructProc(_) => out.push_str("#<struct-procedure>"),
        Value::Foreign(_) => out.push_str("#<foreign>"),
    }
}

fn print_string(out: &mut String, s: &str, escaped: bool) {
    if !escaped {
        out.push_str(s);
        return;
    }
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

fn print_char(out: &mut String, c: char, escaped: bool) {
    if !escaped {
        out.push(c);
        return;
    }
    match c {
        '\n' => out.push_str("#\\newline"),
        '\t' => out.push_str("#\\tab"),
        ' ' => out.push_str("#\\space"),
        '\r' => out.push_str("#\\return"),
        '\u{c}' => out.push_str("#\\page"),
        '\u{8}' => out.push_str("#\\backspace"),
        _ => {
            out.push_str("#\\");
            out.push(c);
        }
    }
}

fn print_pair(it: &Interp, out: &mut String, obj: &Value, escaped: bool) {
    out.push('(');
    // print car
    let (car, mut cdr) = match obj {
        Value::Pair(p) => {
            let p = p.borrow();
            (p.car.clone(), p.cdr.clone())
        }
        _ => unreachable!(),
    };
    print(it, out, &car, escaped);
    // walk the spine
    loop {
        match &cdr {
            Value::Null => break,
            Value::Pair(p) => {
                let (car, next) = {
                    let p = p.borrow();
                    (p.car.clone(), p.cdr.clone())
                };
                out.push(' ');
                print(it, out, &car, escaped);
                cdr = next;
            }
            other => {
                out.push_str(" . ");
                print(it, out, other, escaped);
                break;
            }
        }
    }
    out.push(')');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::Reader;

    fn roundtrip_write(src: &str) -> String {
        let mut it = Interp::new();
        let mut r = Reader::new(src);
        let v = r.read(&mut it).expect("read");
        write_to_string(&it, &v)
    }

    #[test]
    fn write_scalars() {
        let it = Interp::new();
        assert_eq!(write_to_string(&it, &Value::Int(42)), "42");
        assert_eq!(write_to_string(&it, &Value::Bool(true)), "#t");
        assert_eq!(write_to_string(&it, &Value::Null), "()");
        assert_eq!(write_to_string(&it, &Value::Double(2.5)), "2.500000");
        assert_eq!(write_to_string(&it, &Value::Char('a')), "#\\a");
        assert_eq!(write_to_string(&it, &Value::Char('\n')), "#\\newline");
    }

    #[test]
    fn write_vs_display_strings_and_chars() {
        let it = Interp::new();
        let s = Value::make_string("a\"b");
        assert_eq!(write_to_string(&it, &s), "\"a\\\"b\"");
        assert_eq!(display_to_string(&it, &s), "a\"b");
        assert_eq!(display_to_string(&it, &Value::Char('x')), "x");
    }

    #[test]
    fn write_lists() {
        assert_eq!(roundtrip_write("(1 2 3)"), "(1 2 3)");
        assert_eq!(roundtrip_write("(1 . 2)"), "(1 . 2)");
        assert_eq!(roundtrip_write("(1 2 . 3)"), "(1 2 . 3)");
        assert_eq!(roundtrip_write("#(1 2 3)"), "#(1 2 3)");
    }

    #[test]
    fn write_quote_shorthand_prints_as_list() {
        // The reader expands 'x to (quote x); C prints it back as the list form.
        assert_eq!(roundtrip_write("'x"), "(quote x)");
    }

    #[test]
    fn read_write_roundtrip_nested() {
        assert_eq!(
            roundtrip_write("(a (b c) #(1 #\\x \"hi\"))"),
            "(a (b c) #(1 #\\x \"hi\"))"
        );
    }
}
