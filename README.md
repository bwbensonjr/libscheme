# libscheme

A simple Scheme interpreter with an easy-to-integrate API written by
Brent Benson in the early 1990s. A greatly modified version of this
interpreter [became the starting
point](https://www-old.cs.utah.edu/plt/publications/icfp19-fddkmstz.pdf)
for [Racket](https://racket-lang.org/), which currently uses Chez
Scheme as its implementation substrate.

A good explanation of the `libscheme` interpreter is in the paper
*libscheme: Scheme as a C Library* presented at the 1994 USENIX
Symposium on Very High Level Languages (VHLL):

- [`libscheme.pdf`](src/doc/libscheme.pdf)
- [`libscheme.md`](src/doc/libscheme.md)

## Rust port

The original C interpreter lives in [`src/`](src/). A from-scratch
re-implementation in idiomatic, safe Rust lives in
[`crates/`](crates/), preserving the same design principles
(library-first, extensible primitives, per-subsystem init order) and a
module-per-`scheme_*.c` structure:

- [`crates/libscheme`](crates/libscheme) — the interpreter as a library.
  Values are an `enum` (the `Scheme_Object` union), errors and escape
  continuations travel a `Result` channel (the `setjmp`/`longjmp`
  replacement), cyclic data is reclaimed by a tracing GC (the
  [`gc`](https://crates.io/crates/gc) crate, standing in for Boehm GC),
  and the evaluator trampolines tail calls.
- [`crates/scheme`](crates/scheme) — the `scheme` REPL (port of `src/main.c`).
- [`crates/posix`](crates/posix) — a *separate* crate of POSIX bindings,
  demonstrating that an extension can add primitives and nominal types
  through the public API alone (port of `src/posix/`).

```sh
cargo run -p scheme              # the REPL
cargo run -p posix --bin posix_scheme   # the REPL + POSIX extensions
cargo test                       # unit, integration, and the R4RS golden suite
```

Correctness is validated by running Aubrey Jaffer's R4RS `test.scm`
through the interpreter as a golden test: the core suite plus
`(test-sc4)` and `(test-inexact)` pass; `(test-cont)` fails gracefully,
as it does on the C version (libscheme supports escape continuations
only). See [`RUST_PORT_FEASIBILITY.md`](RUST_PORT_FEASIBILITY.md) for the
design rationale and the C-to-Rust mapping.

