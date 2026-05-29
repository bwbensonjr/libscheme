//! Microbenchmarks for the evaluator hot path.
//!
//! Each benchmark builds a standard interpreter and evaluates the workload's
//! `(define ...)` setup *once*, outside the timing loop, leaving only the
//! driving call expression to be timed. This isolates the eval/apply cost
//! (closure framing, body evaluation, variable lookup) from reader and setup
//! overhead.
//!
//! Workloads are kept shallow enough to run on criterion's default-stack
//! thread — the trampoline keeps tail loops flat, and `fib 25` / `tak 18 12 6`
//! nest only a few dozen native frames. Deep recursion and the full R4RS suite
//! are profiled separately via samply on the REPL binary (256 MiB stack).

use criterion::{criterion_group, criterion_main, Criterion};
use libscheme::env::Env;
use libscheme::value::Value;
use libscheme::{Interp, Reader};

/// Build a standard interpreter, evaluate every form in `setup`, and return it
/// alongside the parsed `driver` expression ready to be evaluated repeatedly.
fn prepare(setup: &str, driver: &str) -> (Interp, Value) {
    let mut it = Interp::basic_env();
    let mut r = Reader::new(setup);
    for form in r.read_all(&mut it).expect("read setup") {
        it.eval(form, Env::root()).expect("eval setup");
    }
    let mut dr = Reader::new(driver);
    let expr = dr.read(&mut it).expect("read driver");
    (it, expr)
}

fn bench_eval(c: &mut Criterion, name: &str, setup: &str, driver: &str) {
    let (mut it, expr) = prepare(setup, driver);
    c.bench_function(name, |b| {
        b.iter(|| {
            it.eval(criterion::black_box(expr.clone()), Env::root())
                .expect("eval driver")
        })
    });
}

fn benches(c: &mut Criterion) {
    // Non-tail recursion + arithmetic — the primary closure-call microbench.
    bench_eval(
        c,
        "eval/fib",
        "(define (fib n) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2)))))",
        "(fib 25)",
    );

    // Three-parameter binding stress on closure_frame; heavy non-tail recursion.
    bench_eval(
        c,
        "eval/tak",
        "(define (tak x y z)
           (if (not (< y x))
               z
               (tak (tak (- x 1) y z)
                    (tak (- y 1) z x)
                    (tak (- z 1) x y))))",
        "(tak 18 12 6)",
    );

    // Pure trampoline + per-iteration closure framing, flat native stack.
    bench_eval(
        c,
        "eval/tail-loop",
        "(define (loop n) (if (zero? n) 'done (loop (- n 1))))",
        "(loop 1000000)",
    );

    // List construction (cons) followed by a fold — exercises cons + recursion.
    bench_eval(
        c,
        "eval/list",
        "(define (build n) (if (zero? n) '() (cons n (build (- n 1)))))
         (define (sum lst) (if (null? lst) 0 (+ (car lst) (sum (cdr lst)))))",
        "(sum (build 1000))",
    );
}

criterion_group!(eval, benches);
criterion_main!(eval);
