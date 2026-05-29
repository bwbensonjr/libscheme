//! Proves the libscheme extension API is sufficient for a third-party crate:
//! `posix` registers primitives AND a nominal `<stat>` type through the public
//! API alone, and they work end-to-end through the evaluator.

use libscheme::env::Env;
use libscheme::value::Value;
use libscheme::{write_to_string, Interp, Reader};

/// Build an interpreter with the posix extensions installed (exactly as the
/// `posix_scheme` binary does), evaluate `src`, and return the final result.
fn run(src: &str) -> String {
    let mut it = Interp::basic_env();
    posix::init_posix_file(&mut it);
    posix::init_posix_proc(&mut it);
    let mut r = Reader::new(src);
    let forms = r.read_all(&mut it).expect("read");
    let mut last = Value::Bool(false);
    for f in forms {
        last = it.eval(f, Env::root()).expect("eval");
    }
    write_to_string(&it, &last)
}

#[test]
fn posix_primitives_are_registered() {
    // getcwd returns a non-empty string.
    assert_eq!(run("(string? (posix-getcwd))"), "#t");
    // getpid returns a positive integer.
    assert_eq!(run("(positive? (posix-getpid))"), "#t");
}

#[test]
fn stat_nominal_type_and_accessors() {
    // posix-stat builds a <stat> record; its accessors read fields. We stat the
    // crate's own Cargo.toml, which always exists when tests run.
    let src = "(define s (posix-stat \"Cargo.toml\"))
               (list (> (stat-size s) 0)
                     (number? (stat-mode s))
                     (number? (stat-uid s)))";
    assert_eq!(run(src), "(#t #t #t)");
}

#[test]
fn stat_accessor_rejects_non_stat() {
    // The accessor must error on a non-<stat> argument, proving nominal typing.
    let mut it = Interp::basic_env();
    posix::init_posix_file(&mut it);
    let mut r = Reader::new("(stat-size 42)");
    let form = r.read(&mut it).unwrap();
    assert!(it.eval(form, Env::root()).is_err());
}

#[test]
fn constants_are_bound() {
    // The O_* constants are present and are integers.
    assert_eq!(run("(number? o_rdonly)"), "#t");
    assert_eq!(run("(number? seek_end)"), "#t");
}
