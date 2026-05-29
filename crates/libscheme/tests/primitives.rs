//! Phase 4 acceptance: exercise the typed-subsystem primitives (numbers, lists,
//! strings, chars, vectors, booleans, symbols) end-to-end through the reader and
//! evaluator. These mirror the categories the R4RS `test.scm` covers, scoped to
//! the operations available before Phase 5 (ports/structs/promises).

use libscheme::env::Env;
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

#[test]
fn numbers_arithmetic_and_contagion() {
    assert_eq!(run("(+ 1 2 3 4)"), "10");
    assert_eq!(run("(* 2 3 4)"), "24");
    assert_eq!(run("(- 10 3 2)"), "5");
    assert_eq!(run("(- 5)"), "-5");
    assert_eq!(run("(+ 1 2.5)"), "3.500000"); // int+double => double
    assert_eq!(run("(/ 6 2)"), "3.000000"); // / always double
    assert_eq!(run("(max 3 7 2)"), "7");
    assert_eq!(run("(min 3 7 2)"), "2");
    assert_eq!(run("(abs -4)"), "4");
    assert_eq!(run("(expt 2 10)"), "1024");
}

#[test]
fn numbers_integer_division() {
    assert_eq!(run("(quotient 17 5)"), "3");
    assert_eq!(run("(remainder 17 5)"), "2");
    assert_eq!(run("(modulo 17 5)"), "2");
    assert_eq!(run("(modulo -7 3)"), "2");
    assert_eq!(run("(modulo 7 -3)"), "-2");
    assert_eq!(run("(gcd 12 18)"), "6");
    assert_eq!(run("(lcm 4 6)"), "12");
}

#[test]
fn numbers_predicates_and_comparison() {
    assert_eq!(run("(< 1 2 3)"), "#t");
    assert_eq!(run("(< 1 3 2)"), "#f");
    assert_eq!(run("(= 5 5 5)"), "#t");
    assert_eq!(run("(zero? 0)"), "#t");
    assert_eq!(run("(odd? 3)"), "#t");
    assert_eq!(run("(even? 4)"), "#t");
    assert_eq!(run("(integer? 5)"), "#t");
    assert_eq!(run("(inexact? 5.0)"), "#t");
    assert_eq!(run("(number? 'x)"), "#f");
}

#[test]
fn numbers_rounding_returns_integers() {
    assert_eq!(run("(floor 3.7)"), "3");
    assert_eq!(run("(ceiling 3.2)"), "4");
    assert_eq!(run("(truncate -3.7)"), "-3");
    assert_eq!(run("(round 2.5)"), "2"); // round half toward even-ish (C: > fl+0.5)
    assert_eq!(run("(round 2.6)"), "3");
}

#[test]
fn numbers_string_conversion() {
    assert_eq!(run(r#"(number->string 255 16)"#), "\"ff\"");
    assert_eq!(run(r#"(string->number "42")"#), "42");
    assert_eq!(run(r#"(string->number "3.5")"#), "3.500000");
    assert_eq!(run(r#"(string->number "nope")"#), "#f");
}

#[test]
fn lists_core() {
    assert_eq!(run("(cons 1 2)"), "(1 . 2)");
    assert_eq!(run("(car '(1 2 3))"), "1");
    assert_eq!(run("(cdr '(1 2 3))"), "(2 3)");
    assert_eq!(run("(list 1 2 3)"), "(1 2 3)");
    assert_eq!(run("(length '(a b c d))"), "4");
    assert_eq!(run("(append '(1 2) '(3 4) '(5))"), "(1 2 3 4 5)");
    assert_eq!(run("(reverse '(1 2 3))"), "(3 2 1)");
    assert_eq!(run("(list-ref '(a b c d) 2)"), "c");
    assert_eq!(run("(list-tail '(a b c d) 2)"), "(c d)");
    assert_eq!(run("(cadr '(1 2 3))"), "2");
    assert_eq!(run("(caddr '(1 2 3))"), "3");
}

#[test]
fn lists_predicates_and_search() {
    assert_eq!(run("(pair? '(1))"), "#t");
    assert_eq!(run("(pair? '())"), "#f");
    assert_eq!(run("(null? '())"), "#t");
    assert_eq!(run("(list? '(1 2 3))"), "#t");
    assert_eq!(run("(list? '(1 . 2))"), "#f");
    assert_eq!(run("(memq 'c '(a b c d))"), "(c d)");
    assert_eq!(run("(member 2 '(1 2 3))"), "(2 3)");
    assert_eq!(run("(assq 'b '((a 1) (b 2) (c 3)))"), "(b 2)");
}

#[test]
fn lists_set_car_cdr() {
    assert_eq!(
        run("(define p (cons 1 2)) (set-car! p 99) (set-cdr! p 88) p"),
        "(99 . 88)"
    );
}

#[test]
fn strings() {
    assert_eq!(run(r#"(string-length "hello")"#), "5");
    assert_eq!(run(r#"(string-ref "hello" 1)"#), "#\\e");
    assert_eq!(run(r#"(substring "hello" 1 4)"#), "\"ell\"");
    assert_eq!(run(r#"(string-append "foo" "bar")"#), "\"foobar\"");
    assert_eq!(run(r#"(string=? "abc" "abc")"#), "#t");
    assert_eq!(run(r#"(string<? "abc" "abd")"#), "#t");
    assert_eq!(run(r#"(string-ci=? "ABC" "abc")"#), "#t");
    assert_eq!(run(r#"(string->list "ab")"#), "(#\\a #\\b)");
    assert_eq!(run(r#"(list->string (list #\h #\i))"#), "\"hi\"");
    assert_eq!(run(r#"(make-string 3 #\x)"#), "\"xxx\"");
}

#[test]
fn chars() {
    assert_eq!(run("(char->integer #\\A)"), "65");
    assert_eq!(run("(integer->char 97)"), "#\\a");
    assert_eq!(run("(char-upcase #\\a)"), "#\\A");
    assert_eq!(run("(char-alphabetic? #\\x)"), "#t");
    assert_eq!(run("(char-numeric? #\\5)"), "#t");
    assert_eq!(run("(char<? #\\a #\\b)"), "#t");
    assert_eq!(run("(char-ci=? #\\A #\\a)"), "#t");
}

#[test]
fn vectors() {
    assert_eq!(run("(vector 1 2 3)"), "#(1 2 3)");
    assert_eq!(run("(make-vector 3 0)"), "#(0 0 0)");
    assert_eq!(run("(vector-length #(a b c))"), "3");
    assert_eq!(run("(vector-ref #(a b c) 1)"), "b");
    assert_eq!(run("(vector->list #(1 2 3))"), "(1 2 3)");
    assert_eq!(run("(list->vector '(1 2 3))"), "#(1 2 3)");
    assert_eq!(
        run("(define v (vector 1 2 3)) (vector-set! v 0 99) v"),
        "#(99 2 3)"
    );
}

#[test]
fn booleans_and_equivalence() {
    assert_eq!(run("(not #f)"), "#t");
    assert_eq!(run("(not 5)"), "#f");
    assert_eq!(run("(boolean? #t)"), "#t");
    assert_eq!(run("(eq? 'a 'a)"), "#t");
    assert_eq!(run("(eqv? 1.5 1.5)"), "#t");
    assert_eq!(run("(equal? '(1 2 (3)) '(1 2 (3)))"), "#t");
    assert_eq!(run("(eq? '(1) '(1))"), "#f"); // distinct pairs
}

#[test]
fn symbols() {
    assert_eq!(run("(symbol? 'foo)"), "#t");
    assert_eq!(run(r#"(symbol->string 'hello)"#), "\"hello\"");
    // string->symbol interns, so it is eq? to the literal symbol.
    assert_eq!(run(r#"(eq? (string->symbol "xyz") 'xyz)"#), "#t");
}
