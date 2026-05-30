//! The read-eval-print loop — the Rust analogue of `main.c`.
//!
//! Same shape as the C driver: build the env, load any command-line files, then
//! enter `read -> eval (catching errors) -> write` at the prompt. Errors and
//! stray escape continuations are caught at this boundary (the
//! `SCHEME_CATCH_ERROR` analogue, main.c:61) and reported without killing the
//! loop. A leading `#!` line in a loaded file is skipped, as in C (main.c:43).
//!
//! The actual driving runs on a worker thread with a large stack: a safe-Rust
//! tree-walker recurses on the native stack for non-tail calls, and a Rust
//! stack overflow aborts uncatchably — so [`run`] spawns the work with generous
//! headroom (the plan's deliberate, behavior-preserving safety net).

use crate::env::Env;
use crate::error::SchemeError;
use crate::printer::write_to_string;
use crate::reader::Reader;
use crate::value::Value;
use crate::Interp;
use std::io::{self, BufRead, Write};

/// Stack size for the evaluation worker thread (256 MiB).
const EVAL_STACK_SIZE: usize = 256 * 1024 * 1024;

/// Build a standard interpreter, run `setup` to install any extensions, then
/// load the given files and enter the REPL — all on a large-stack worker thread.
/// `setup` is where a driver (e.g. the posix binary) registers its primitives,
/// exactly like `init_posix_*` in `posix/main.c`.
pub fn run_with<F>(files: Vec<String>, setup: F)
where
    F: FnOnce(&mut Interp) + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .stack_size(EVAL_STACK_SIZE)
        .spawn(move || {
            let mut it = Interp::basic_env();
            setup(&mut it);
            load_files(&mut it, &files);
            repl(&mut it);
        })
        .expect("spawn eval thread");
    handle.join().expect("eval thread panicked");
}

/// Convenience: run the plain interpreter with no extensions.
pub fn run(files: Vec<String>) {
    run_with(files, |_it| {});
}

/// Load and evaluate every form in each file, reporting (but not stopping on)
/// errors — the command-line loading loop of main.c:37.
pub fn load_files(it: &mut Interp, files: &[String]) {
    for path in files {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("could not open file for loading: {path}: {e}");
                continue;
            }
        };
        // Skip a leading `#!` line if present (main.c:43).
        let src = strip_shebang(&contents);
        let forms = {
            let mut r = Reader::new(src);
            match r.read_all(it) {
                Ok(forms) => forms,
                Err(e) => {
                    report_error(it, &e);
                    continue;
                }
            }
        };
        for form in forms {
            if let Err(e) = it.eval(form, Env::root()) {
                report_error(it, &e);
            }
        }
    }
}

/// The interactive loop. Reads one datum from stdin — spanning as many lines as
/// the datum needs — evaluates it, and writes the result.
fn repl(it: &mut Interp) {
    println!("libscheme Scheme interpreter");
    let stdin = io::stdin();
    let mut buffer = String::new();
    loop {
        print!("> ");
        let _ = io::stdout().flush();
        buffer.clear();
        // Accumulate lines until the reader can parse one complete datum. A
        // form that spans several lines (an unclosed list, string, etc.) leaves
        // the reader `incomplete`, so we print a continuation prompt and read
        // more rather than reporting a spurious "end of file" error.
        let form = loop {
            match stdin.lock().read_line(&mut buffer) {
                Ok(0) => {
                    println!("\n; done");
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("read error: {e}");
                    return;
                }
            }
            let mut r = Reader::new(&buffer);
            match r.read(it) {
                Ok(form) => break form,
                Err(e) if r.incomplete() => {
                    // Datum continues on the next line — keep the buffer and
                    // prompt for more.
                    let _ = e;
                    print!("  ");
                    let _ = io::stdout().flush();
                    continue;
                }
                Err(e) => {
                    report_error(it, &e);
                    break Value::Eof;
                }
            }
        };
        match form {
            Value::Eof => continue,
            form => match it.eval(form, Env::root()) {
                Ok(v) => println!("{}", write_to_string(it, &v)),
                Err(e) => report_error(it, &e),
            },
        }
    }
}

/// Report an error at the REPL boundary without aborting the loop.
fn report_error(it: &Interp, e: &SchemeError) {
    match e {
        SchemeError::User(u) => {
            if u.irritants.is_empty() {
                eprintln!("error: {}", u.message);
            } else {
                let irritants: Vec<String> =
                    u.irritants.iter().map(|v| write_to_string(it, v)).collect();
                eprintln!("error: {} {}", u.message, irritants.join(" "));
            }
        }
        SchemeError::ContinuationInvoked { .. } => {
            eprintln!("error: continuation invoked outside its dynamic extent");
        }
    }
}

/// Strip a leading `#!...` line (shebang) from source, like the C `fscanf`.
fn strip_shebang(src: &str) -> &str {
    if let Some(rest) = src.strip_prefix("#!") {
        match rest.find('\n') {
            Some(nl) => &rest[nl + 1..],
            None => "",
        }
    } else {
        src
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shebang_is_stripped() {
        assert_eq!(strip_shebang("#!/usr/bin/scheme\n(+ 1 2)"), "(+ 1 2)");
        assert_eq!(strip_shebang("(+ 1 2)"), "(+ 1 2)");
        assert_eq!(strip_shebang("#!only-a-shebang"), "");
    }

    #[test]
    fn load_files_evaluates_and_binds() {
        // Write a temp file, load it, confirm a global it defines is visible.
        let dir = std::env::temp_dir();
        let path = dir.join("libscheme_repl_test.scm");
        std::fs::write(&path, "#!shebang\n(define answer 42)\n").unwrap();
        let mut it = Interp::basic_env();
        load_files(&mut it, &[path.to_string_lossy().into_owned()]);
        let sym = it.intern("answer");
        assert!(it.lookup_global(sym).unwrap().eq(&Value::Int(42)));
        let _ = std::fs::remove_file(&path);
    }
}
