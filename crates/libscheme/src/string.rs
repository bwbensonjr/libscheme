//! String primitives — the Rust analogue of `scheme_string.c`.
//!
//! Strings are mutable (`string-set!`, `string-fill!`), held in
//! `Gc<GcCell<String>>`. Indexing is by character, matching the ASCII test
//! suite; comparisons use byte/char ordering as C's `strcmp` does. Case-
//! insensitive comparisons fold with ASCII upper-casing like C's `toupper`.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Value};
use std::cmp::Ordering;

pub fn init(it: &mut Interp) {
    let str_ty = it.make_type("<string>");
    it.register_value("<string>", Value::TypeObject(str_ty));

    it.register("string?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Str(_))))
    });
    it.register("make-string", Arity::Range(1, 2), |_it, a| {
        let len = int_arg(&a[0], "make-string")?;
        let fill = match a.get(1) {
            Some(Value::Char(c)) => *c,
            Some(_) => return Err(SchemeError::msg("make-string: second arg must be a character")),
            None => ' ',
        };
        Ok(Value::make_string(fill.to_string().repeat(len as usize)))
    });
    it.register("string", Arity::AtLeast(0), |_it, a| {
        let mut s = String::new();
        for v in a {
            match v {
                Value::Char(c) => s.push(*c),
                _ => return Err(SchemeError::msg("string: args must all be characters")),
            }
        }
        Ok(Value::make_string(s))
    });
    it.register("string-length", Arity::Exact(1), |_it, a| {
        Ok(Value::Int(with_str(&a[0], "string-length")?.chars().count() as i64))
    });
    it.register("string-ref", Arity::Exact(2), |_it, a| {
        let s = with_str(&a[0], "string-ref")?;
        let i = int_arg(&a[1], "string-ref")? as usize;
        s.chars()
            .nth(i)
            .map(Value::Char)
            .ok_or_else(|| SchemeError::msg("string-ref: index out of range"))
    });
    it.register("string-set!", Arity::Exact(3), |_it, a| {
        let i = int_arg(&a[1], "string-set!")? as usize;
        let c = match &a[2] {
            Value::Char(c) => *c,
            _ => return Err(SchemeError::msg("string-set!: third arg must be a character")),
        };
        match &a[0] {
            Value::Str(s) => {
                let mut chars: Vec<char> = s.borrow().chars().collect();
                if i >= chars.len() {
                    return Err(SchemeError::msg("string-set!: index out of range"));
                }
                chars[i] = c;
                *s.borrow_mut() = chars.into_iter().collect();
                Ok(a[0].clone())
            }
            _ => Err(SchemeError::msg("string-set!: first arg must be a string")),
        }
    });

    // comparisons.
    it.register("string=?", Arity::Exact(2), |_it, a| str_cmp(a, "string=?", false, |o| o == Ordering::Equal));
    it.register("string<?", Arity::Exact(2), |_it, a| str_cmp(a, "string<?", false, |o| o == Ordering::Less));
    it.register("string>?", Arity::Exact(2), |_it, a| str_cmp(a, "string>?", false, |o| o == Ordering::Greater));
    it.register("string<=?", Arity::Exact(2), |_it, a| str_cmp(a, "string<=?", false, |o| o != Ordering::Greater));
    it.register("string>=?", Arity::Exact(2), |_it, a| str_cmp(a, "string>=?", false, |o| o != Ordering::Less));
    it.register("string-ci=?", Arity::Exact(2), |_it, a| str_cmp(a, "string-ci=?", true, |o| o == Ordering::Equal));
    it.register("string-ci<?", Arity::Exact(2), |_it, a| str_cmp(a, "string-ci<?", true, |o| o == Ordering::Less));
    it.register("string-ci>?", Arity::Exact(2), |_it, a| str_cmp(a, "string-ci>?", true, |o| o == Ordering::Greater));
    it.register("string-ci<=?", Arity::Exact(2), |_it, a| str_cmp(a, "string-ci<=?", true, |o| o != Ordering::Greater));
    it.register("string-ci>=?", Arity::Exact(2), |_it, a| str_cmp(a, "string-ci>=?", true, |o| o != Ordering::Less));

    it.register("substring", Arity::Exact(3), |_it, a| {
        let s = with_str(&a[0], "substring")?;
        let chars: Vec<char> = s.chars().collect();
        let start = int_arg(&a[1], "substring")? as usize;
        let finish = int_arg(&a[2], "substring")? as usize;
        if start > chars.len() || finish > chars.len() || start > finish {
            return Err(SchemeError::msg("substring: index out of bounds"));
        }
        Ok(Value::make_string(chars[start..finish].iter().collect::<String>()))
    });
    it.register("string-append", Arity::AtLeast(0), |_it, a| {
        let mut s = String::new();
        for v in a {
            s.push_str(&with_str(v, "string-append")?);
        }
        Ok(Value::make_string(s))
    });
    it.register("string->list", Arity::Exact(1), |_it, a| {
        let chars: Vec<Value> = with_str(&a[0], "string->list")?.chars().map(Value::Char).collect();
        Ok(Value::list(&chars))
    });
    it.register("list->string", Arity::Exact(1), |_it, a| {
        let items = a[0]
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("list->string: arg must be a list"))?;
        let mut s = String::new();
        for v in items {
            match v {
                Value::Char(c) => s.push(c),
                _ => return Err(SchemeError::msg("list->string: all elements must be characters")),
            }
        }
        Ok(Value::make_string(s))
    });
    it.register("string-copy", Arity::Exact(1), |_it, a| {
        Ok(Value::make_string(with_str(&a[0], "string-copy")?))
    });
    it.register("string-fill!", Arity::Exact(2), |_it, a| {
        let c = match &a[1] {
            Value::Char(c) => *c,
            _ => return Err(SchemeError::msg("string-fill!: second arg must be a character")),
        };
        match &a[0] {
            Value::Str(s) => {
                let n = s.borrow().chars().count();
                *s.borrow_mut() = c.to_string().repeat(n);
                Ok(a[0].clone())
            }
            _ => Err(SchemeError::msg("string-fill!: first arg must be a string")),
        }
    });
}

fn with_str(v: &Value, who: &str) -> SchemeResult<String> {
    match v {
        Value::Str(s) => Ok(s.borrow().clone()),
        _ => Err(SchemeError::msg(format!("{who}: arg must be a string"))),
    }
}

fn int_arg(v: &Value, who: &str) -> SchemeResult<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        _ => Err(SchemeError::msg(format!("{who}: expected an integer"))),
    }
}

fn str_cmp(args: &[Value], who: &str, ci: bool, pred: fn(Ordering) -> bool) -> SchemeResult {
    let a = with_str(&args[0], who)?;
    let b = with_str(&args[1], who)?;
    let ord = if ci {
        a.to_ascii_uppercase().cmp(&b.to_ascii_uppercase())
    } else {
        a.cmp(&b)
    };
    Ok(Value::Bool(pred(ord)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_set_mutates() {
        let mut it = Interp::new();
        super::init(&mut it);
        let s = Value::make_string("abc");
        let sym = it.intern("string-set!");
        let set = it.lookup_global(sym).unwrap();
        let _ = it.apply(set, &[s.clone(), Value::Int(1), Value::Char('X')]).unwrap();
        match &s {
            Value::Str(inner) => assert_eq!(*inner.borrow(), "aXc"),
            _ => unreachable!(),
        }
    }
}
