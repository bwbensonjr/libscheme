//! Phase 5 acceptance: promises (delay/force), structs (define-struct), string
//! ports (read/write round-trips), and the escape-continuation limitation
//! (upward continuations must fail *gracefully*, matching C's (test-cont)).

use libscheme::env::Env;
use libscheme::error::SchemeError;
use libscheme::value::Value;
use libscheme::{write_to_string, Interp, Reader};

fn run(src: &str) -> String {
    let mut it = Interp::basic_env();
    let mut r = Reader::new(src);
    let forms = r.read_all(&mut it).expect("read");
    let mut last = Value::Bool(false);
    for f in forms {
        last = it.eval(f, Env::root()).expect("eval");
    }
    write_to_string(&it, &last)
}

/// Evaluate, expecting an error; return the error message.
fn run_err(src: &str) -> String {
    let mut it = Interp::basic_env();
    let mut r = Reader::new(src);
    let forms = r.read_all(&mut it).expect("read");
    let mut result = Ok(Value::Bool(false));
    for f in forms {
        result = it.eval(f, Env::root());
        if result.is_err() {
            break;
        }
    }
    match result {
        Err(SchemeError::User(e)) => e.message,
        Err(SchemeError::ContinuationInvoked { .. }) => "continuation-escaped".to_string(),
        Ok(_) => panic!("expected an error, got a value"),
    }
}

#[test]
fn promises_force_and_memoize() {
    assert_eq!(run("(force (delay (+ 1 2)))"), "3");
    // force is memoized: a side-effecting thunk runs only once.
    let src = "(define count 0)
               (define p (delay (begin (set! count (+ count 1)) 'val)))
               (force p)
               (force p)
               count";
    assert_eq!(run(src), "1");
    // force of a non-promise returns it unchanged.
    assert_eq!(run("(force 42)"), "42");
}

#[test]
fn structs_constructor_predicate_accessors_mutators() {
    let src = "(define-struct point (x y))
               (define p (make-point 3 4))
               (list (point? p) (point-x p) (point-y p))";
    assert_eq!(run(src), "(#t 3 4)");

    // Mutator updates a field in place.
    let src2 = "(define-struct cell (val))
                (define c (make-cell 10))
                (set-cell-val! c 99)
                (cell-val c)";
    assert_eq!(run(src2), "99");

    // Predicate distinguishes nominal types.
    let src3 = "(define-struct a (x))
                (define-struct b (x))
                (list (a? (make-a 1)) (a? (make-b 1)))";
    assert_eq!(run(src3), "(#t #f)");
}

#[test]
fn string_ports_read() {
    // read pulls successive data from a string input port.
    let src = "(define p (open-input-string \"42 (a b) foo\"))
               (list (read p) (read p) (read p))";
    assert_eq!(run(src), "(42 (a b) foo)");
    // read past the end yields eof.
    assert_eq!(run("(eof-object? (read (open-input-string \"\")))"), "#t");
}

#[test]
fn read_char_and_peek_char() {
    let src = "(define p (open-input-string \"ab\"))
               (list (peek-char p) (read-char p) (read-char p) (eof-object? (read-char p)))";
    assert_eq!(run(src), "(#\\a #\\a #\\b #t)");
}

#[test]
fn write_and_display_to_string() {
    assert_eq!(run(r#"(write-to-string "hi")"#), "\"\\\"hi\\\"\"");
    assert_eq!(run(r#"(display-to-string "hi")"#), "\"hi\"");
}

#[test]
fn with_input_from_string_swaps_current_port() {
    let src = "(with-input-from-string \"99\" (lambda () (read)))";
    assert_eq!(run(src), "99");
}

#[test]
fn port_predicates() {
    assert_eq!(run("(input-port? (open-input-string \"x\"))"), "#t");
    assert_eq!(run("(output-port? (current-output-port))"), "#t");
    assert_eq!(run("(input-port? 5)"), "#f");
}

#[test]
fn call_cc_escape_still_works_with_io() {
    // An escape continuation used within the normal dynamic extent.
    let src = "(call/cc (lambda (k) (+ 1 (k 10) 100)))";
    assert_eq!(run(src), "10");
}

#[test]
fn upward_continuation_fails_gracefully() {
    // Capture a continuation, return it, then invoke it after call/cc returned.
    // C's (test-cont) relies on upward continuations, which are unsupported;
    // here that must surface as an error, NOT a crash or UB.
    let src = "(define k #f)
               (+ 1 (call/cc (lambda (c) (set! k c) 1)))
               (k 99)";
    let msg = run_err(src);
    // The escaped-continuation token reaches the top level.
    assert_eq!(msg, "continuation-escaped");
}
