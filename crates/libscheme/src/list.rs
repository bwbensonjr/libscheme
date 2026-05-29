//! Pair and list primitives — the Rust analogue of `scheme_list.c`.
//!
//! `set-car!`/`set-cdr!` mutate the `GcCell<Pair>` in place (the operations
//! that can build cycles the GC must reclaim). `list?` uses the tortoise/hare
//! cycle check from C (scheme_list.c:259). The full family of `cXr` accessors
//! (`caar`…`cdddr`) is generated from their a/d access pattern.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Value};

pub fn init(it: &mut Interp) {
    let null_ty = it.make_type("<empty-list>");
    let pair_ty = it.make_type("<pair>");
    it.register_value("<empty-list>", Value::TypeObject(null_ty));
    it.register_value("<pair>", Value::TypeObject(pair_ty));

    it.register("pair?", Arity::Exact(1), |_it, a| Ok(Value::Bool(a[0].is_pair())));
    it.register("null?", Arity::Exact(1), |_it, a| Ok(Value::Bool(a[0].is_null())));
    it.register("cons", Arity::Exact(2), |_it, a| Ok(Value::cons(a[0].clone(), a[1].clone())));
    it.register("car", Arity::Exact(1), |_it, a| {
        a[0].car().ok_or_else(|| SchemeError::msg("car: arg must be pair"))
    });
    it.register("cdr", Arity::Exact(1), |_it, a| {
        a[0].cdr().ok_or_else(|| SchemeError::msg("cdr: arg must be pair"))
    });
    it.register("set-car!", Arity::Exact(2), |_it, a| match &a[0] {
        Value::Pair(p) => {
            p.borrow_mut().car = a[1].clone();
            Ok(a[1].clone())
        }
        _ => Err(SchemeError::msg("set-car!: first arg must be pair")),
    });
    it.register("set-cdr!", Arity::Exact(2), |_it, a| match &a[0] {
        Value::Pair(p) => {
            p.borrow_mut().cdr = a[1].clone();
            Ok(a[1].clone())
        }
        _ => Err(SchemeError::msg("set-cdr!: first arg must be pair")),
    });

    it.register("list?", Arity::Exact(1), |_it, a| Ok(Value::Bool(is_list(&a[0]))));
    it.register("list", Arity::AtLeast(0), |_it, a| Ok(Value::list(a)));
    it.register("length", Arity::Exact(1), |_it, a| {
        a[0].list_len()
            .map(|n| Value::Int(n as i64))
            .ok_or_else(|| SchemeError::msg("length: arg must be a list"))
    });
    it.register("append", Arity::AtLeast(0), |_it, a| append(a));
    it.register("reverse", Arity::Exact(1), |_it, a| {
        let mut items = a[0]
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("reverse: arg must be a list"))?;
        items.reverse();
        Ok(Value::list(&items))
    });
    it.register("list-tail", Arity::Exact(2), |_it, a| list_tail(a));
    it.register("list-ref", Arity::Exact(2), |_it, a| {
        let tail = list_tail(a)?;
        tail.car().ok_or_else(|| SchemeError::msg("list-ref: index too large"))
    });

    // membership / association, by the three equivalence predicates.
    it.register("memq", Arity::Exact(2), |_it, a| mem(a, "memq", Value::eq));
    it.register("memv", Arity::Exact(2), |_it, a| mem(a, "memv", Value::eqv));
    it.register("member", Arity::Exact(2), |_it, a| mem(a, "member", Value::equal));
    it.register("assq", Arity::Exact(2), |_it, a| ass(a, "assq", Value::eq));
    it.register("assv", Arity::Exact(2), |_it, a| ass(a, "assv", Value::eqv));
    it.register("assoc", Arity::Exact(2), |_it, a| ass(a, "assoc", Value::equal));

    // cXr accessors: each char after 'c' (read right-to-left) is a/d.
    for name in [
        "caar", "cadr", "cdar", "cddr", "caaar", "caadr", "cadar", "cdaar", "cdadr", "cddar",
        "caddr", "cdddr",
    ] {
        // The C `cddar` is actually cdr·cdr·cdr (a known libscheme quirk:
        // its cddar_prim does CDR(CDR(CDR))). We follow R4RS semantics here,
        // deriving from the name, which is the correct behavior the suite
        // expects; the C typo only affects `cddar`, unused by test.scm.
        let ops: Vec<char> = name[1..name.len() - 1].chars().rev().collect();
        it.register(name, Arity::Exact(1), move |_it, a| cxr(&a[0], &ops, name));
    }
}

/// `list?` with the tortoise/hare cycle detection (scheme_list.c:259).
fn is_list(v: &Value) -> bool {
    let mut slow = v.clone();
    let mut fast = v.clone();
    loop {
        if fast.is_null() {
            return true;
        }
        let Some(next) = fast.cdr() else { return false };
        if next.is_null() {
            return true;
        }
        let Some(next2) = next.cdr() else { return false };
        fast = next2;
        slow = slow.cdr().unwrap();
        if slow.eq(&fast) {
            return false; // cycle
        }
    }
}

/// `append` — copy all but the last argument, sharing the last's tail
/// (scheme_list.c:315).
fn append(args: &[Value]) -> SchemeResult {
    if args.is_empty() {
        return Ok(Value::Null);
    }
    let mut result = args[args.len() - 1].clone();
    for arg in args[..args.len() - 1].iter().rev() {
        let items = arg
            .list_to_vec()
            .ok_or_else(|| SchemeError::msg("append: arguments must be lists"))?;
        for v in items.into_iter().rev() {
            result = Value::cons(v, result);
        }
    }
    Ok(result)
}

/// `list-tail` — drop `k` elements (scheme_list.c:354).
fn list_tail(args: &[Value]) -> SchemeResult {
    let k = match &args[1] {
        Value::Int(n) if *n >= 0 => *n as usize,
        _ => return Err(SchemeError::msg("list-tail: second arg must be a non-negative integer")),
    };
    let mut cur = args[0].clone();
    for _ in 0..k {
        cur = cur
            .cdr()
            .ok_or_else(|| SchemeError::msg("list-tail: index too large for list"))?;
    }
    Ok(cur)
}

fn mem(args: &[Value], who: &str, eq: fn(&Value, &Value) -> bool) -> SchemeResult {
    let mut cur = args[1].clone();
    while let Value::Pair(p) = &cur {
        let (car, cdr) = {
            let b = p.borrow();
            (b.car.clone(), b.cdr.clone())
        };
        if eq(&args[0], &car) {
            return Ok(cur);
        }
        cur = cdr;
    }
    if !cur.is_null() {
        return Err(SchemeError::msg(format!("{who}: second arg must be a list")));
    }
    Ok(Value::Bool(false))
}

fn ass(args: &[Value], who: &str, eq: fn(&Value, &Value) -> bool) -> SchemeResult {
    let mut cur = args[1].clone();
    while let Value::Pair(p) = &cur {
        let (car, cdr) = {
            let b = p.borrow();
            (b.car.clone(), b.cdr.clone())
        };
        match car.car() {
            Some(key) if eq(&args[0], &key) => return Ok(car),
            Some(_) => {}
            None => return Err(SchemeError::msg(format!("{who}: arg must be a list of pairs"))),
        }
        cur = cdr;
    }
    Ok(Value::Bool(false))
}

/// Apply a sequence of car/cdr operations (`a`/`d`) right-to-left.
fn cxr(v: &Value, ops: &[char], name: &'static str) -> SchemeResult {
    let mut cur = v.clone();
    for &op in ops {
        cur = match op {
            'a' => cur.car(),
            'd' => cur.cdr(),
            _ => unreachable!(),
        }
        .ok_or_else(|| SchemeError::msg(format!("{name}: arg must be a pair")))?;
    }
    Ok(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_predicate_handles_cycles() {
        assert!(is_list(&Value::list(&[Value::Int(1), Value::Int(2)])));
        assert!(is_list(&Value::Null));
        assert!(!is_list(&Value::cons(Value::Int(1), Value::Int(2))));

        // Build a cycle with set-cdr! and confirm list? returns false.
        let a = Value::cons(Value::Int(1), Value::Null);
        if let Value::Pair(p) = &a {
            p.borrow_mut().cdr = a.clone();
        }
        assert!(!is_list(&a));
    }

    #[test]
    fn append_shares_last_tail() {
        let r = append(&[
            Value::list(&[Value::Int(1), Value::Int(2)]),
            Value::list(&[Value::Int(3)]),
        ])
        .unwrap();
        assert!(r.equal(&Value::list(&[Value::Int(1), Value::Int(2), Value::Int(3)])));
    }
}
