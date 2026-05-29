//! Character primitives — the Rust analogue of `scheme_char.c`.
//!
//! Predicates and case operations use ASCII semantics (C's `isalpha`/`toupper`
//! etc.). `char->integer`/`integer->char` round-trip code points; the suite
//! only exercises ASCII, so a Rust `char` is a faithful stand-in for C's byte.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Value};

pub fn init(it: &mut Interp) {
    let char_ty = it.make_type("<char>");
    it.register_value("<char>", Value::TypeObject(char_ty));

    it.register("char?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Char(_))))
    });

    it.register("char=?", Arity::Exact(2), |_it, a| char_cmp(a, "char=?", false, |x, y| x == y));
    it.register("char<?", Arity::Exact(2), |_it, a| char_cmp(a, "char<?", false, |x, y| x < y));
    it.register("char>?", Arity::Exact(2), |_it, a| char_cmp(a, "char>?", false, |x, y| x > y));
    it.register("char<=?", Arity::Exact(2), |_it, a| char_cmp(a, "char<=?", false, |x, y| x <= y));
    it.register("char>=?", Arity::Exact(2), |_it, a| char_cmp(a, "char>=?", false, |x, y| x >= y));
    it.register("char-ci=?", Arity::Exact(2), |_it, a| char_cmp(a, "char-ci=?", true, |x, y| x == y));
    it.register("char-ci<?", Arity::Exact(2), |_it, a| char_cmp(a, "char-ci<?", true, |x, y| x < y));
    it.register("char-ci>?", Arity::Exact(2), |_it, a| char_cmp(a, "char-ci>?", true, |x, y| x > y));
    it.register("char-ci<=?", Arity::Exact(2), |_it, a| char_cmp(a, "char-ci<=?", true, |x, y| x <= y));
    it.register("char-ci>=?", Arity::Exact(2), |_it, a| char_cmp(a, "char-ci>=?", true, |x, y| x >= y));

    it.register("char-alphabetic?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(ch(&a[0], "char-alphabetic?")?.is_ascii_alphabetic()))
    });
    it.register("char-numeric?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(ch(&a[0], "char-numeric?")?.is_ascii_digit()))
    });
    it.register("char-whitespace?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(ch(&a[0], "char-whitespace?")?.is_ascii_whitespace()))
    });
    it.register("char-upper-case?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(ch(&a[0], "char-upper-case?")?.is_ascii_uppercase()))
    });
    it.register("char-lower-case?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(ch(&a[0], "char-lower-case?")?.is_ascii_lowercase()))
    });

    it.register("char->integer", Arity::Exact(1), |_it, a| {
        Ok(Value::Int(ch(&a[0], "char->integer")? as i64))
    });
    it.register("integer->char", Arity::Exact(1), |_it, a| match &a[0] {
        Value::Int(n) => char::from_u32(*n as u32)
            .map(Value::Char)
            .ok_or_else(|| SchemeError::msg("integer->char: not a valid code point")),
        _ => Err(SchemeError::msg("integer->char: arg must be an integer")),
    });
    it.register("char-upcase", Arity::Exact(1), |_it, a| {
        Ok(Value::Char(ch(&a[0], "char-upcase")?.to_ascii_uppercase()))
    });
    it.register("char-downcase", Arity::Exact(1), |_it, a| {
        Ok(Value::Char(ch(&a[0], "char-downcase")?.to_ascii_lowercase()))
    });
}

fn ch(v: &Value, who: &str) -> SchemeResult<char> {
    match v {
        Value::Char(c) => Ok(*c),
        _ => Err(SchemeError::msg(format!("{who}: arg must be a character"))),
    }
}

fn char_cmp(args: &[Value], who: &str, ci: bool, pred: fn(char, char) -> bool) -> SchemeResult {
    let mut a = ch(&args[0], who)?;
    let mut b = ch(&args[1], who)?;
    if ci {
        a = a.to_ascii_uppercase();
        b = b.to_ascii_uppercase();
    }
    Ok(Value::Bool(pred(a, b)))
}
