//! Phase 1 gate: round-trip the canonical `write-test-obj` from the R4RS suite
//! (`src/test.scm:844`). This is the object the original test suite uses to
//! exercise the reader and both printer modes, so matching it end-to-end is the
//! agreed acceptance check for the reader/printer phase.

use libscheme::{display_to_string, write_to_string, Interp, Reader};

/// The reader must accept `write-test-obj`, and `write` must reproduce it
/// verbatim (re-readable form, with escaped strings and `#\a` characters).
#[test]
fn write_test_obj_roundtrips_under_write() {
    let src = r#"(#t #f #\a () 9739 -3 . #((test) "te \" \" st" "" test #() b c))"#;
    let mut it = Interp::new();
    let mut r = Reader::new(src);
    let obj = r.read(&mut it).expect("read write-test-obj");
    assert_eq!(write_to_string(&it, &obj), src);
}

/// `display` renders strings and characters raw (no quotes, no `#\`). Note the
/// empty string `""` contributes nothing, so it leaves *two* adjacent spaces
/// between `st` and `test` — this matches the C printer exactly (the suite's
/// `display-test-obj` literal is a separate datum, not a string-equality
/// oracle, so we assert the faithful byte-for-byte output here).
#[test]
fn write_test_obj_display_matches_c() {
    let src = r#"(#t #f #\a () 9739 -3 . #((test) "te \" \" st" "" test #() b c))"#;
    let expected_display = r#"(#t #f a () 9739 -3 . #((test) te " " st  test #() b c))"#;
    let mut it = Interp::new();
    let mut r = Reader::new(src);
    let obj = r.read(&mut it).expect("read write-test-obj");
    assert_eq!(display_to_string(&it, &obj), expected_display);
}

/// `load-test-obj` = `(define foo (quote <write-test-obj>))` must also survive a
/// read → write round-trip, exercising nested quote/define structure.
#[test]
fn load_test_obj_roundtrips() {
    let src = r#"(define foo (quote (#t #f #\a () 9739 -3 . #((test) "te \" \" st" "" test #() b c))))"#;
    let mut it = Interp::new();
    let mut r = Reader::new(src);
    let obj = r.read(&mut it).expect("read load-test-obj");
    assert_eq!(write_to_string(&it, &obj), src);
}
