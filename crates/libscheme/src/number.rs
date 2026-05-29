//! The numeric tower — the Rust analogue of `scheme_number.c` (+ the promotion
//! rules from `scheme_nummacs.h`).
//!
//! Like C, the tower has exactly two rungs: exact integers (`i64`, the C `int`)
//! and inexact doubles (`f64`). There are no bignums, rationals, or complex
//! numbers, though the predicates `complex?`/`real?`/`rational?` exist for
//! R4RS compatibility (mapping to "is a number" / "is an integer", as in C).
//!
//! Exact/inexact contagion matches C: an operation on two integers stays exact;
//! if either operand is a double the result is a double. `/` always yields a
//! double (even `int/int`), exactly as `bin_div` does. `floor`/`ceiling`/
//! `truncate`/`round` return an *integer* even for double input — faithful to
//! C's `scheme_make_integer(floor(...))`.

use crate::error::{SchemeError, SchemeResult};
use crate::interp::Interp;
use crate::value::{Arity, Value};

/// One number, decoded from a [`Value`]. Mirrors the int/double union arms.
#[derive(Copy, Clone)]
enum Num {
    Int(i64),
    Dbl(f64),
}

impl Num {
    fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Dbl(d) => d,
        }
    }
    fn to_value(self) -> Value {
        match self {
            Num::Int(i) => Value::Int(i),
            Num::Dbl(d) => Value::Double(d),
        }
    }
}

fn num(v: &Value, who: &str) -> SchemeResult<Num> {
    match v {
        Value::Int(i) => Ok(Num::Int(*i)),
        Value::Double(d) => Ok(Num::Dbl(*d)),
        _ => Err(SchemeError::msg(format!("{who}: arg must be a number"))),
    }
}

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Double(_))
}

/// `scheme_init_number`: numeric type objects + all numeric primitives.
pub fn init(it: &mut Interp) {
    let int_ty = it.make_type("<integer>");
    let dbl_ty = it.make_type("<double>");
    it.register_value("<integer>", Value::TypeObject(int_ty));
    it.register_value("<double>", Value::TypeObject(dbl_ty));

    // Predicates. number?/complex?/real? accept any number; integer?/rational?/
    // exact? accept only integers; inexact? accepts only doubles (scheme_number.c:159).
    it.register("number?", Arity::Exact(1), |_it, a| Ok(Value::Bool(is_number(&a[0]))));
    it.register("complex?", Arity::Exact(1), |_it, a| Ok(Value::Bool(is_number(&a[0]))));
    it.register("real?", Arity::Exact(1), |_it, a| Ok(Value::Bool(is_number(&a[0]))));
    it.register("rational?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Int(_))))
    });
    it.register("integer?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Int(_))))
    });
    it.register("exact?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Int(_))))
    });
    it.register("inexact?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(matches!(a[0], Value::Double(_))))
    });

    // n-ary comparisons (scheme_number.c:214). Compared numerically with
    // promotion, pairwise across all args.
    it.register("=", Arity::AtLeast(1), |_it, a| nary_cmp(a, "=", |x, y| x == y));
    it.register("<", Arity::AtLeast(1), |_it, a| nary_cmp(a, "<", |x, y| x < y));
    it.register(">", Arity::AtLeast(1), |_it, a| nary_cmp(a, ">", |x, y| x > y));
    it.register("<=", Arity::AtLeast(1), |_it, a| nary_cmp(a, "<=", |x, y| x <= y));
    it.register(">=", Arity::AtLeast(1), |_it, a| nary_cmp(a, ">=", |x, y| x >= y));

    // sign / parity predicates.
    it.register("zero?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(num(&a[0], "zero?")?.as_f64() == 0.0))
    });
    it.register("positive?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(num(&a[0], "positive?")?.as_f64() > 0.0))
    });
    it.register("negative?", Arity::Exact(1), |_it, a| {
        Ok(Value::Bool(num(&a[0], "negative?")?.as_f64() < 0.0))
    });
    it.register("odd?", Arity::Exact(1), |_it, a| match &a[0] {
        Value::Int(i) => Ok(Value::Bool(i % 2 != 0)),
        Value::Double(_) => Ok(Value::Bool(false)),
        _ => Err(SchemeError::msg("odd?: arg must be a number")),
    });
    it.register("even?", Arity::Exact(1), |_it, a| match &a[0] {
        Value::Int(i) => Ok(Value::Bool(i % 2 == 0)),
        Value::Double(_) => Ok(Value::Bool(false)),
        _ => Err(SchemeError::msg("even?: arg must be a number")),
    });

    // arithmetic.
    it.register("+", Arity::AtLeast(0), |_it, a| fold(a, "+", Num::Int(0), bin_add));
    it.register("*", Arity::AtLeast(0), |_it, a| fold(a, "*", Num::Int(1), bin_mul));
    it.register("-", Arity::AtLeast(1), |_it, a| minus(a));
    it.register("/", Arity::AtLeast(1), |_it, a| divide(a));
    it.register("max", Arity::AtLeast(1), |_it, a| twoary(a, "max", bin_max));
    it.register("min", Arity::AtLeast(1), |_it, a| twoary(a, "min", bin_min));
    it.register("abs", Arity::Exact(1), |_it, a| match num(&a[0], "abs")? {
        Num::Int(i) => Ok(Value::Int(i.abs())),
        Num::Dbl(d) => Ok(Value::Double(d.abs())),
    });

    it.register("quotient", Arity::Exact(2), |_it, a| int_div(a, "quotient", |x, y| x / y));
    it.register("remainder", Arity::Exact(2), |_it, a| int_div(a, "remainder", |x, y| x % y));
    it.register("modulo", Arity::Exact(2), |_it, a| modulo(a));
    it.register("gcd", Arity::AtLeast(0), |_it, a| fold(a, "gcd", Num::Int(0), bin_gcd));
    it.register("lcm", Arity::AtLeast(0), |_it, a| fold(a, "lcm", Num::Int(1), bin_lcm));

    // rounding — always returns an integer, matching C.
    it.register("floor", Arity::Exact(1), |_it, a| round_op(a, "floor", f64::floor));
    it.register("ceiling", Arity::Exact(1), |_it, a| round_op(a, "ceiling", f64::ceil));
    it.register("truncate", Arity::Exact(1), |_it, a| round_op(a, "truncate", f64::trunc));
    it.register("round", Arity::Exact(1), |_it, a| round_op(a, "round", round_half_up));

    // transcendental — all return doubles.
    it.register("exp", Arity::Exact(1), |_it, a| unary_f(a, "exp", f64::exp));
    it.register("log", Arity::Exact(1), |_it, a| unary_f(a, "log", f64::ln));
    it.register("sin", Arity::Exact(1), |_it, a| unary_f(a, "sin", f64::sin));
    it.register("cos", Arity::Exact(1), |_it, a| unary_f(a, "cos", f64::cos));
    it.register("asin", Arity::Exact(1), |_it, a| unary_f(a, "asin", f64::asin));
    it.register("acos", Arity::Exact(1), |_it, a| unary_f(a, "acos", f64::acos));
    it.register("atan", Arity::Range(1, 2), |_it, a| {
        let y = num(&a[0], "atan")?.as_f64();
        if a.len() == 2 {
            let x = num(&a[1], "atan")?.as_f64();
            Ok(Value::Double(y.atan2(x)))
        } else {
            Ok(Value::Double(y.atan()))
        }
    });
    it.register("sqrt", Arity::Exact(1), |_it, a| unary_f(a, "sqrt", f64::sqrt));
    it.register("expt", Arity::Exact(2), |_it, a| {
        // C uses pow over doubles; preserve exact^nonneg-int as exact for the
        // common integer case so (expt 2 10) stays an integer.
        match (&a[0], &a[1]) {
            (Value::Int(b), Value::Int(e)) if *e >= 0 => {
                Ok(Value::Int(b.pow(*e as u32)))
            }
            _ => {
                let b = num(&a[0], "expt")?.as_f64();
                let e = num(&a[1], "expt")?.as_f64();
                Ok(Value::Double(b.powf(e)))
            }
        }
    });

    it.register("exact->inexact", Arity::Exact(1), |_it, a| {
        Ok(Value::Double(num(&a[0], "exact->inexact")?.as_f64()))
    });
    it.register("inexact->exact", Arity::Exact(1), |_it, a| match num(&a[0], "inexact->exact")? {
        Num::Int(i) => Ok(Value::Int(i)),
        Num::Dbl(d) => Ok(Value::Int(d as i64)),
    });

    it.register("number->string", Arity::Range(1, 2), number_to_string);
    it.register("string->number", Arity::Range(1, 2), string_to_number);
}

// --- arithmetic helpers ---

fn bin_add(a: Num, b: Num) -> Num {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Num::Int(x + y),
        _ => Num::Dbl(a.as_f64() + b.as_f64()),
    }
}
fn bin_mul(a: Num, b: Num) -> Num {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Num::Int(x * y),
        _ => Num::Dbl(a.as_f64() * b.as_f64()),
    }
}
fn bin_sub(a: Num, b: Num) -> Num {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Num::Int(x - y),
        _ => Num::Dbl(a.as_f64() - b.as_f64()),
    }
}
fn bin_max(a: Num, b: Num) -> Num {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Num::Int(x.max(y)),
        _ => Num::Dbl(a.as_f64().max(b.as_f64())),
    }
}
fn bin_min(a: Num, b: Num) -> Num {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Num::Int(x.min(y)),
        _ => Num::Dbl(a.as_f64().min(b.as_f64())),
    }
}
fn bin_gcd(a: Num, b: Num) -> Num {
    let (x, y) = (a.as_f64().abs() as i64, b.as_f64().abs() as i64);
    let g = gcd_i64(x, y);
    match (a, b) {
        (Num::Int(_), Num::Int(_)) => Num::Int(g),
        _ => Num::Dbl(g as f64),
    }
}
fn bin_lcm(a: Num, b: Num) -> Num {
    let (x, y) = (a.as_f64().abs() as i64, b.as_f64().abs() as i64);
    let l = if x == 0 || y == 0 { 0 } else { x / gcd_i64(x, y) * y };
    match (a, b) {
        (Num::Int(_), Num::Int(_)) => Num::Int(l),
        _ => Num::Dbl(l as f64),
    }
}

fn gcd_i64(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn fold(args: &[Value], who: &str, identity: Num, op: fn(Num, Num) -> Num) -> SchemeResult {
    let mut acc = identity;
    for (i, v) in args.iter().enumerate() {
        let n = num(v, who)?;
        if i == 0 {
            acc = n;
        } else {
            acc = op(acc, n);
        }
    }
    // For empty args, `acc` is the identity; for one arg, it's that arg.
    Ok(acc.to_value())
}

fn minus(args: &[Value]) -> SchemeResult {
    let first = num(&args[0], "-")?;
    if args.len() == 1 {
        return Ok(bin_sub(Num::Int(0), first).to_value());
    }
    let mut acc = first;
    for v in &args[1..] {
        acc = bin_sub(acc, num(v, "-")?);
    }
    Ok(acc.to_value())
}

fn divide(args: &[Value]) -> SchemeResult {
    // `/` always produces a double, matching C's bin_div.
    let first = num(&args[0], "/")?.as_f64();
    if args.len() == 1 {
        return Ok(Value::Double(1.0 / first));
    }
    let mut acc = first;
    for v in &args[1..] {
        acc /= num(v, "/")?.as_f64();
    }
    Ok(Value::Double(acc))
}

fn twoary(args: &[Value], who: &str, op: fn(Num, Num) -> Num) -> SchemeResult {
    let mut acc = num(&args[0], who)?;
    for v in &args[1..] {
        acc = op(acc, num(v, who)?);
    }
    Ok(acc.to_value())
}

fn nary_cmp(args: &[Value], who: &str, op: fn(f64, f64) -> bool) -> SchemeResult {
    for w in args.windows(2) {
        let a = num(&w[0], who)?.as_f64();
        let b = num(&w[1], who)?.as_f64();
        if !op(a, b) {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn int_div(args: &[Value], who: &str, op: fn(i64, i64) -> i64) -> SchemeResult {
    // C coerces doubles to int for quotient/remainder; result is int if both
    // were int, else double.
    let n1 = num(&args[0], who)?;
    let n2 = num(&args[1], who)?;
    let (i1, i2) = (n1.as_f64() as i64, n2.as_f64() as i64);
    if i2 == 0 {
        return Err(SchemeError::msg(format!("{who}: division by zero")));
    }
    let r = op(i1, i2);
    match (n1, n2) {
        (Num::Int(_), Num::Int(_)) => Ok(Value::Int(r)),
        _ => Ok(Value::Double(r as f64)),
    }
}

fn modulo(args: &[Value]) -> SchemeResult {
    let n1 = num(&args[0], "modulo")?;
    let n2 = num(&args[1], "modulo")?;
    let (i1, i2) = (n1.as_f64() as i64, n2.as_f64() as i64);
    if i2 == 0 {
        return Err(SchemeError::msg("modulo: division by zero"));
    }
    let i = i1 % i2;
    // Adjust toward the divisor's sign (scheme_number.c:481).
    let adjusted = if (i2 < 0 && i > 0) || (i2 > 0 && i < 0) { i + i2 } else { i };
    // C always returns an integer for modulo.
    Ok(Value::Int(adjusted))
}

fn round_op(args: &[Value], who: &str, f: fn(f64) -> f64) -> SchemeResult {
    match num(&args[0], who)? {
        Num::Int(i) => Ok(Value::Int(i)),
        Num::Dbl(d) => Ok(Value::Int(f(d) as i64)),
    }
}

/// C's `round`: round half up (val > floor+0.5 ? ceil : floor).
fn round_half_up(val: f64) -> f64 {
    let fl = val.floor();
    if val > fl + 0.5 {
        val.ceil()
    } else {
        fl
    }
}

fn unary_f(args: &[Value], who: &str, f: fn(f64) -> f64) -> SchemeResult {
    Ok(Value::Double(f(num(&args[0], who)?.as_f64())))
}

fn number_to_string(_it: &mut Interp, args: &[Value]) -> SchemeResult {
    let radix = match args.get(1) {
        Some(Value::Int(r)) => *r,
        Some(_) => return Err(SchemeError::msg("number->string: radix must be an integer")),
        None => 10,
    };
    match &args[0] {
        Value::Int(i) => {
            let s = match radix {
                2 => format!("{i:b}"),
                8 => format!("{i:o}"),
                10 => format!("{i}"),
                16 => format!("{i:x}"),
                _ => return Err(SchemeError::msg("number->string: radix must be 2, 8, 10 or 16")),
            };
            Ok(Value::make_string(s))
        }
        Value::Double(d) => Ok(Value::make_string(format!("{d:.6}"))),
        _ => Err(SchemeError::msg("number->string: arg must be a number")),
    }
}

fn string_to_number(_it: &mut Interp, args: &[Value]) -> SchemeResult {
    let s = match &args[0] {
        Value::Str(s) => s.borrow().clone(),
        _ => return Err(SchemeError::msg("string->number: first arg must be a string")),
    };
    let base = match args.get(1) {
        Some(Value::Int(b)) => *b as u32,
        Some(_) => return Err(SchemeError::msg("string->number: radix must be an integer")),
        None => 10,
    };
    if s.is_empty() {
        return Ok(Value::Bool(false));
    }
    let is_float = s.contains(['.', 'e', 'E']);
    if base == 10 && is_float {
        match s.parse::<f64>() {
            Ok(d) => Ok(Value::Double(d)),
            Err(_) => Ok(Value::Bool(false)),
        }
    } else {
        match i64::from_str_radix(&s, base) {
            Ok(n) => Ok(Value::Int(n)),
            Err(_) => Ok(Value::Bool(false)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(args: &[Value], f: fn(&mut Interp, &[Value]) -> SchemeResult) -> Value {
        let mut it = Interp::new();
        f(&mut it, args).unwrap()
    }

    #[test]
    fn contagion() {
        // int + int = int; int + double = double
        assert!(one(&[Value::Int(2), Value::Int(3)], |_it, a| fold(a, "+", Num::Int(0), bin_add))
            .eq(&Value::Int(5)));
        match one(&[Value::Int(2), Value::Double(0.5)], |_it, a| fold(a, "+", Num::Int(0), bin_add)) {
            Value::Double(d) => assert_eq!(d, 2.5),
            _ => panic!("expected double"),
        }
    }

    #[test]
    fn division_always_double() {
        match divide(&[Value::Int(6), Value::Int(2)]) {
            Ok(Value::Double(d)) => assert_eq!(d, 3.0),
            other => panic!("expected double 3.0, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn modulo_follows_divisor_sign() {
        // (modulo 7 -3) => -2 ; (modulo -7 3) => 2
        assert!(modulo(&[Value::Int(7), Value::Int(-3)]).unwrap().eq(&Value::Int(-2)));
        assert!(modulo(&[Value::Int(-7), Value::Int(3)]).unwrap().eq(&Value::Int(2)));
        // remainder keeps the dividend's sign.
        assert!(int_div(&[Value::Int(7), Value::Int(-3)], "remainder", |x, y| x % y)
            .unwrap()
            .eq(&Value::Int(1)));
    }

    #[test]
    fn floor_returns_integer() {
        assert!(round_op(&[Value::Double(3.7)], "floor", f64::floor).unwrap().eq(&Value::Int(3)));
        assert!(round_op(&[Value::Double(-3.2)], "floor", f64::floor).unwrap().eq(&Value::Int(-4)));
    }
}
