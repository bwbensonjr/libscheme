//! Phase 3 acceptance: exercise every special form end-to-end. Special attention
//! to the riskiest pieces called out in the plan — quasiquote nesting/splicing,
//! named-let tail recursion, `do` loops, `cond` with `=>`, and `defmacro`.

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
fn cond_basic_and_else() {
    assert_eq!(run("(cond (#f 1) (#t 2) (else 3))"), "2");
    assert_eq!(run("(cond (#f 1) (else 3))"), "3");
    assert_eq!(run("(cond (#f 1))"), "#f");
    // a bare-test clause yields the test value
    assert_eq!(run("(cond (42))"), "42");
}

#[test]
fn cond_arrow() {
    // (cond (test => proc)) applies proc to the test value.
    assert_eq!(run("(cond ((memq 'b '(a b c)) => car) (else 'no))"), "b");
}

#[test]
fn case_form() {
    let src = "(case (* 2 3)
                 ((2 3 5 7) 'prime)
                 ((1 4 6 8 9) 'composite))";
    assert_eq!(run(src), "composite");
    assert_eq!(run("(case 'x ((a) 1) (else 'other))"), "other");
}

#[test]
fn and_or_short_circuit() {
    assert_eq!(run("(and 1 2 3)"), "3");
    assert_eq!(run("(and 1 #f 3)"), "#f");
    assert_eq!(run("(and)"), "#t");
    assert_eq!(run("(or #f #f 5)"), "5");
    assert_eq!(run("(or #f #f)"), "#f");
    assert_eq!(run("(or)"), "#f");
    // short-circuit: the unbound symbol after the truthy value is never evaluated
    assert_eq!(run("(or 1 undefined-symbol)"), "1");
    assert_eq!(run("(and #f undefined-symbol)"), "#f");
}

#[test]
fn let_forms() {
    assert_eq!(run("(let ((x 1) (y 2)) (+ x y))"), "3");
    // let does NOT see its own bindings in the inits
    assert_eq!(run("(let ((x 5)) (let ((x 10) (y x)) y))"), "5");
    // let* sees earlier bindings
    assert_eq!(run("(let* ((x 5) (y (* x 2))) y)"), "10");
    // letrec for mutual recursion
    let src = "(letrec ((even? (lambda (n) (if (zero? n) #t (odd? (- n 1)))))
                        (odd?  (lambda (n) (if (zero? n) #f (even? (- n 1))))))
                 (even? 88))";
    assert_eq!(run(src), "#t");
}

#[test]
fn let_with_internal_defines() {
    let src = "(let ((x 10))
                 (define (double n) (* n 2))
                 (double x))";
    assert_eq!(run(src), "20");
}

#[test]
fn named_let_tail_recursive() {
    // Named let summing 1..n; large n must not overflow (tail call).
    let src = "(let loop ((i 0) (acc 0))
                 (if (= i 1000000)
                     acc
                     (loop (+ i 1) (+ acc 1))))";
    assert_eq!(run(src), "1000000");
}

#[test]
fn do_loop() {
    // classic do-loop vector sum
    let src = "(do ((i 0 (+ i 1))
                     (sum 0 (+ sum i)))
                    ((= i 5) sum))";
    assert_eq!(run(src), "10"); // 0+1+2+3+4
    // do with a step-less variable
    let src2 = "(do ((i 0 (+ i 1)) (x 'fixed)) ((= i 3) x))";
    assert_eq!(run(src2), "fixed");
}

#[test]
fn begin_sequencing() {
    assert_eq!(run("(begin (define a 1) (define b 2) (+ a b))"), "3");
}

#[test]
fn quasiquote_basic() {
    assert_eq!(run("`(1 2 3)"), "(1 2 3)");
    assert_eq!(run("`(1 ,(+ 1 1) 3)"), "(1 2 3)");
    assert_eq!(run("(let ((x 5)) `(a ,x c))"), "(a 5 c)");
}

#[test]
fn quasiquote_splicing() {
    assert_eq!(run("`(1 ,@(list 2 3) 4)"), "(1 2 3 4)");
    assert_eq!(run("(let ((xs '(b c))) `(a ,@xs d))"), "(a b c d)");
    // splice at the end
    assert_eq!(run("`(1 ,@(list 2 3))"), "(1 2 3)");
}

#[test]
fn quasiquote_nested() {
    // A nested quasiquote: the inner unquote is preserved one level down.
    assert_eq!(run("`(a `(b ,(+ 1 2)))"), "(a (quasiquote (b (unquote (+ 1 2)))))");
    // But an unquote at the outer level inside the nest IS evaluated.
    assert_eq!(run("`(a `(b ,,(+ 1 2)))"), "(a (quasiquote (b (unquote 3))))");
}

#[test]
fn quasiquote_vector() {
    assert_eq!(run("`#(1 ,(+ 1 1) 3)"), "#(1 2 3)");
}

#[test]
fn defmacro_expands() {
    // A simple swap-args macro and a (my-if) macro.
    let src = "(defmacro my-list (a b) (list 'list a b))
               (my-list 1 2)";
    assert_eq!(run(src), "(1 2)");

    let src2 = "(defmacro unless (test . body)
                  (list 'if test #f (cons 'begin body)))
                (unless #f 'ran)";
    assert_eq!(run(src2), "ran");
}

#[test]
fn delay_makes_a_promise() {
    // force lands in Phase 5; here we just confirm delay yields a promise object.
    assert_eq!(run("(delay (+ 1 2))"), "#<promise>");
}
