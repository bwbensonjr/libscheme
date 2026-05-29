//! Golden acceptance test: run Aubrey Jaffer's R4RS `test.scm` (the same suite
//! the C libscheme is checked against) through the Rust interpreter and assert
//! it reports no errors.
//!
//! `test.scm` accumulates any failures into a global `errs` list (and prints
//! "Passed all tests" from `report-errs` when it is empty). We load the suite,
//! run the optional `(test-sc4)` and `(test-inexact)` batteries, and assert
//! `errs` is still null. `(test-cont)` is asserted *separately* to fail
//! gracefully — libscheme supports only escape continuations, so the upward
//! continuation it needs surfaces as an error rather than a crash, matching the
//! C version where `(test-cont)` is documented as always failing.
//!
//! The suite does its own file I/O (`call-with-input-file "test.scm"`, writing
//! `tmp1`/`tmp2`/`tmp3`), so the test runs with the current directory set to the
//! fixtures folder. It is the sole test in this file so that the process-global
//! cwd change cannot race other tests.

use libscheme::env::Env;
use libscheme::value::Value;
use libscheme::{Interp, Reader};

/// Evaluate one expression string in the given interpreter at top level.
fn eval_str(it: &mut Interp, src: &str) -> Result<Value, libscheme::SchemeError> {
    let mut r = Reader::new(src);
    let form = r.read(it).expect("read");
    it.eval(form, Env::root())
}

/// True if the suite's `errs` global is the empty list.
fn errs_empty(it: &mut Interp) -> bool {
    match eval_str(it, "errs").expect("errs is bound after loading test.scm") {
        Value::Null => true,
        other => {
            // Print the accumulated errors to aid debugging on failure.
            let rendered = libscheme::write_to_string(it, &other);
            eprintln!("test.scm recorded errors: {rendered}");
            false
        }
    }
}

#[test]
fn r4rs_test_suite_passes() {
    let fixtures = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    std::env::set_current_dir(&fixtures).expect("cd to fixtures");

    let mut it = Interp::basic_env();

    // Load the suite (runs the IEEE-required core tests as a side effect).
    eval_str(&mut it, r#"(load "test.scm")"#).expect("load test.scm");
    assert!(errs_empty(&mut it), "core R4RS tests must pass");

    // Optional batteries: R4RS-only procedures and inexact arithmetic.
    eval_str(&mut it, "(test-sc4)").expect("run test-sc4");
    assert!(errs_empty(&mut it), "(test-sc4) must pass");

    eval_str(&mut it, "(test-inexact)").expect("run test-inexact");
    assert!(errs_empty(&mut it), "(test-inexact) must pass");

    // (test-cont) needs an UPWARD continuation, which libscheme does not
    // support (escape-only). It must fail GRACEFULLY: invoking the escaped
    // continuation surfaces as an error in the Result channel, never a crash.
    match eval_str(&mut it, "(test-cont)") {
        Err(libscheme::SchemeError::ContinuationInvoked { .. }) => {
            // The expected outcome: the upward continuation escaped to top level.
        }
        Err(libscheme::SchemeError::User(_)) => {
            // Also acceptable — surfaced as a user-visible error, not UB.
        }
        Ok(_) => {
            panic!("(test-cont) unexpectedly succeeded — upward continuations are unsupported")
        }
    }
}
