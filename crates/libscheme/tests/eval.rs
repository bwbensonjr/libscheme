//! Phase 2 acceptance: evaluate real Scheme programs end-to-end through the
//! core evaluator with only the irreducible special forms (`quote`, `if`,
//! `lambda`, `define`, `set!`, `begin`) plus the `fun.rs` primitives. The typed
//! primitives (`+`, `car`, …) land in Phase 4, so these tests register small
//! helper primitives where arithmetic is needed.

use libscheme::env::Env;
use libscheme::error::SchemeError;
use libscheme::value::{Arity, Value};
use libscheme::{write_to_string, Interp, Reader};

/// Evaluate every form in `src` against a `basic_env` augmented with a few
/// arithmetic primitives, returning the `write` form of the final result.
fn run(src: &str) -> String {
    let mut it = Interp::basic_env();
    install_arith(&mut it);
    let mut r = Reader::new(src);
    let forms = r.read_all(&mut it).expect("read");
    let mut last = Value::Bool(false);
    for f in forms {
        last = it.eval(f, Env::root()).expect("eval");
    }
    write_to_string(&it, &last)
}

/// Minimal integer primitives so the eval tests can do arithmetic before the
/// number subsystem (Phase 4) exists.
fn install_arith(it: &mut Interp) {
    it.register("+", Arity::AtLeast(0), |_it, args| {
        let mut sum = 0i64;
        for a in args {
            match a {
                Value::Int(n) => sum += n,
                _ => return Err(SchemeError::msg("+: not an int")),
            }
        }
        Ok(Value::Int(sum))
    });
    it.register("-", Arity::AtLeast(1), |_it, args| {
        let mut acc = match &args[0] {
            Value::Int(n) => *n,
            _ => return Err(SchemeError::msg("-: not an int")),
        };
        if args.len() == 1 {
            return Ok(Value::Int(-acc));
        }
        for a in &args[1..] {
            match a {
                Value::Int(n) => acc -= n,
                _ => return Err(SchemeError::msg("-: not an int")),
            }
        }
        Ok(Value::Int(acc))
    });
    it.register("*", Arity::AtLeast(0), |_it, args| {
        let mut p = 1i64;
        for a in args {
            match a {
                Value::Int(n) => p *= n,
                _ => return Err(SchemeError::msg("*: not an int")),
            }
        }
        Ok(Value::Int(p))
    });
    it.register("=", Arity::Exact(2), |_it, args| {
        Ok(Value::Bool(args[0].eq(&args[1])))
    });
    it.register("<", Arity::Exact(2), |_it, args| match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        _ => Err(SchemeError::msg("<: not ints")),
    });
    it.register("zero?", Arity::Exact(1), |_it, args| {
        Ok(Value::Bool(matches!(&args[0], Value::Int(0))))
    });
}

#[test]
fn self_evaluating_and_quote() {
    assert_eq!(run("42"), "42");
    assert_eq!(run("'foo"), "foo");
    assert_eq!(run("'(1 2 3)"), "(1 2 3)");
    assert_eq!(run("#t"), "#t");
}

#[test]
fn if_and_begin() {
    assert_eq!(run("(if #t 1 2)"), "1");
    assert_eq!(run("(if #f 1 2)"), "2");
    assert_eq!(run("(if (zero? 0) 'yes 'no)"), "yes");
    assert_eq!(run("(begin 1 2 3)"), "3");
}

#[test]
fn define_and_lookup() {
    assert_eq!(run("(define x 10) (+ x 5)"), "15");
    assert_eq!(run("(define (sq n) (* n n)) (sq 9)"), "81");
}

#[test]
fn lambda_closures_capture_env() {
    let src = "
        (define (adder n) (lambda (x) (+ x n)))
        (define add5 (adder 5))
        (add5 100)";
    assert_eq!(run(src), "105");
}

#[test]
fn set_bang_mutates() {
    let src = "
        (define counter 0)
        (define (bump) (set! counter (+ counter 1)))
        (bump) (bump) (bump)
        counter";
    assert_eq!(run(src), "3");
}

#[test]
fn rest_arguments() {
    // (lambda args ...) collects all args into a list.
    assert_eq!(run("((lambda args args) 1 2 3)"), "(1 2 3)");
    // (lambda (a . rest) ...) binds a then the rest.
    assert_eq!(run("((lambda (a . rest) rest) 1 2 3)"), "(2 3)");
}

#[test]
fn internal_defines_are_letrec_scoped() {
    // Mutually recursive internal defines must see each other.
    let src = "
        (define (classify n)
          (define (even? k) (if (zero? k) #t (odd? (- k 1))))
          (define (odd? k) (if (zero? k) #f (even? (- k 1))))
          (even? n))
        (classify 10)";
    assert_eq!(run(src), "#t");
}

#[test]
fn procedure_predicate_and_apply() {
    assert_eq!(run("(procedure? +)"), "#t");
    assert_eq!(run("(procedure? 5)"), "#f");
    assert_eq!(run("(apply + '(1 2 3 4))"), "10");
    assert_eq!(run("(apply + 1 2 '(3 4))"), "10");
}

#[test]
fn map_and_for_each() {
    assert_eq!(run("(map (lambda (x) (* x x)) '(1 2 3 4))"), "(1 4 9 16)");
    assert_eq!(run("(map + '(1 2 3) '(10 20 30))"), "(11 22 33)");
}

#[test]
fn call_cc_escape() {
    // Escape continuation: return early from a computation.
    let src = "
        (call/cc (lambda (k)
          (+ 1 (k 42))))";
    assert_eq!(run(src), "42");
    // Without invoking k, the normal value flows through.
    assert_eq!(run("(call/cc (lambda (k) (+ 1 2)))"), "3");
}

#[test]
fn tail_calls_do_not_overflow() {
    // A deep tail-recursive countdown. With the trampoline this runs in
    // constant native stack; without TCO it would overflow.
    let src = "
        (define (loop n) (if (zero? n) 'done (loop (- n 1))))
        (loop 1000000)";
    assert_eq!(run(src), "done");
}

#[test]
fn unbound_symbol_errors() {
    let mut it = Interp::basic_env();
    let mut r = Reader::new("nonexistent");
    let form = r.read(&mut it).unwrap();
    match it.eval(form, Env::root()) {
        Err(SchemeError::User(e)) => assert!(e.message.contains("unbound")),
        Err(_) => panic!("expected an unbound-symbol user error"),
        Ok(_) => panic!("expected an error, got a value"),
    }
}
